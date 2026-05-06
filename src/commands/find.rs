use crate::cli::{AddArgs, FindArgs};
use crate::ui;
use anyhow::{Context, Result};
use console::style;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType},
};
use serde::Deserialize;
use std::io::{stdout, IsTerminal, Write};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const SEARCH_API_BASE: &str = "https://skills.sh";
const USER_AGENT: &str = concat!("ateam/", env!("CARGO_PKG_VERSION"));

#[derive(Deserialize)]
struct SearchResponse {
    skills: Vec<SearchSkill>,
}

#[derive(Deserialize, Clone)]
struct SearchSkill {
    id: String,
    name: String,
    #[serde(default)]
    installs: u64,
    #[serde(default)]
    source: String,
}

pub fn run(args: FindArgs, no_sync: bool) -> Result<()> {
    let query = args.query.join(" ");

    if !query.is_empty() {
        return run_non_interactive(&query);
    }

    let stdin_is_tty = std::io::stdin().is_terminal();
    let stdout_is_tty = std::io::stdout().is_terminal();
    if !stdin_is_tty || !stdout_is_tty {
        ui::plain(format!(
            "{}",
            style("Tip: if running in a coding agent, follow these steps:").dim()
        ));
        ui::plain(format!(
            "{}",
            style("  1) ateam find [query]").dim()
        ));
        ui::plain(format!(
            "{}",
            style("  2) ateam add <owner/repo> --skill <name>").dim()
        ));
        return Ok(());
    }

    match run_interactive_picker()? {
        None => {
            ui::plain(format!("{}", style("Search cancelled").dim()));
            Ok(())
        }
        Some(skill) => install_selected(skill, no_sync),
    }
}

fn run_non_interactive(query: &str) -> Result<()> {
    let skills = search_blocking(query).context("calling skills.sh search API")?;

    if skills.is_empty() {
        ui::plain(format!("No skills found for \"{}\"", query));
        return Ok(());
    }

    ui::plain(format!(
        "{} npx skills add <owner/repo@skill>",
        style("Install with").dim()
    ));
    ui::plain("");

    for skill in skills.iter().take(6) {
        let pkg = if skill.source.is_empty() {
            &skill.id
        } else {
            &skill.source
        };
        let installs_str = format_installs(skill.installs);
        let installs_part = if installs_str.is_empty() {
            String::new()
        } else {
            format!(" {}", style(installs_str).cyan())
        };
        ui::plain(format!("{}@{}{}", pkg, skill.name, installs_part));
        ui::plain(format!(
            "{}",
            style(format!("└ https://skills.sh/{}", skill.id)).dim()
        ));
        ui::plain("");
    }

    Ok(())
}

fn install_selected(skill: SearchSkill, no_sync: bool) -> Result<()> {
    let pkg = if skill.source.is_empty() {
        skill.id.clone()
    } else {
        skill.source.clone()
    };

    ui::plain("");
    ui::plain(format!(
        "Installing {} from {}...",
        style(&skill.name).bold(),
        style(&pkg).dim()
    ));
    ui::plain("");

    let add_args = AddArgs {
        source: pkg,
        list: false,
        skill: vec![skill.name.clone()],
        all: false,
        agents: vec![],
        yes: false,
        global: false,
        profile: vec![],
        project: None,
        r#ref: None,
        copy: false,
        dangerously_accept_openclaw_risks: false,
    };
    crate::commands::add::run(add_args, no_sync)?;

    ui::plain("");
    ui::plain(format!(
        "{} {}",
        style("Discover more skills at").dim(),
        style(format!("https://skills.sh/{}", skill.id))
    ));
    ui::plain("");
    Ok(())
}

// ---------------------------------------------------------------------------
// Search

fn search_blocking(query: &str) -> Result<Vec<SearchSkill>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("building reqwest client")?;
    let resp: SearchResponse = client
        .get(format!("{}/api/search", SEARCH_API_BASE))
        .query(&[("q", query), ("limit", "10")])
        .send()
        .context("calling skills.sh search API")?
        .json()
        .context("parsing skills.sh search response")?;
    let mut skills = resp.skills;
    skills.sort_by(|a, b| b.installs.cmp(&a.installs));
    Ok(skills)
}

// ---------------------------------------------------------------------------
// Interactive picker
//
// Mirrors vercel-labs/skills `runSearchPrompt`: raw stdin, debounced
// search-as-you-type, fzf-style list with arrow nav, enter to select.
// Debounce window shortens as the query lengthens (Vercel's heuristic:
// 350 - len*50, floor 150ms) so longer queries feel snappier.

struct PickerState {
    query: String,
    results: Vec<SearchSkill>,
    selected: usize,
    loading: bool,
    last_lines: u16,
    pending_at: Option<Instant>,
    in_flight_for: Option<String>,
}

enum SearchMsg {
    Result {
        query: String,
        skills: Vec<SearchSkill>,
    },
    Failed {
        query: String,
    },
}

fn run_interactive_picker() -> Result<Option<SearchSkill>> {
    let (req_tx, req_rx) = mpsc::channel::<String>();
    let (res_tx, res_rx) = mpsc::channel::<SearchMsg>();

    thread::spawn(move || {
        while let Ok(initial) = req_rx.recv() {
            let mut q = initial;
            // Drain any queued requests — only the latest matters.
            while let Ok(newer) = req_rx.try_recv() {
                q = newer;
            }
            match search_blocking(&q) {
                Ok(skills) => {
                    let _ = res_tx.send(SearchMsg::Result { query: q, skills });
                }
                Err(_) => {
                    let _ = res_tx.send(SearchMsg::Failed { query: q });
                }
            }
        }
    });

    terminal::enable_raw_mode().context("entering raw mode")?;
    let mut out = stdout();
    execute!(out, cursor::Hide)?;

    let result = picker_loop(&mut out, &req_tx, &res_rx);

    let _ = execute!(out, cursor::Show);
    let _ = terminal::disable_raw_mode();

    drop(req_tx);
    result
}

fn picker_loop(
    out: &mut impl Write,
    req_tx: &mpsc::Sender<String>,
    res_rx: &mpsc::Receiver<SearchMsg>,
) -> Result<Option<SearchSkill>> {
    let mut state = PickerState {
        query: String::new(),
        results: Vec::new(),
        selected: 0,
        loading: false,
        last_lines: 0,
        pending_at: None,
        in_flight_for: None,
    };

    render(out, &mut state)?;

    loop {
        // Poll for keystrokes with a short timeout so we can also
        // service debounce expiry and incoming search results.
        if event::poll(Duration::from_millis(40))? {
            match event::read()? {
                Event::Key(KeyEvent { kind, .. }) if kind == KeyEventKind::Release => {
                    // crossterm on Windows emits Release events; we only act on Press/Repeat.
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Esc, ..
                }) => return Ok(None),
                Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers,
                    ..
                }) if modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    ..
                }) => {
                    return Ok(state.results.get(state.selected).cloned());
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Up, ..
                }) => {
                    state.selected = state.selected.saturating_sub(1);
                    render(out, &mut state)?;
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Down,
                    ..
                }) => {
                    if !state.results.is_empty() {
                        state.selected =
                            (state.selected + 1).min(state.results.len().saturating_sub(1));
                    }
                    render(out, &mut state)?;
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                }) => {
                    if !state.query.is_empty() {
                        state.query.pop();
                        state.pending_at = Some(Instant::now());
                        render(out, &mut state)?;
                    }
                }
                Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers,
                    ..
                }) if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && (' '..='~').contains(&c) =>
                {
                    state.query.push(c);
                    state.pending_at = Some(Instant::now());
                    render(out, &mut state)?;
                }
                _ => {}
            }
        }

        // Debounce: if the query has settled long enough, dispatch a search.
        if let Some(at) = state.pending_at {
            let len = state.query.len() as u64;
            let debounce_ms = 350u64.saturating_sub(len * 50).max(150);
            if at.elapsed() >= Duration::from_millis(debounce_ms) {
                state.pending_at = None;
                if state.query.len() < 2 {
                    state.results.clear();
                    state.selected = 0;
                    state.loading = false;
                    state.in_flight_for = None;
                    render(out, &mut state)?;
                } else {
                    state.loading = true;
                    state.in_flight_for = Some(state.query.clone());
                    let _ = req_tx.send(state.query.clone());
                    render(out, &mut state)?;
                }
            }
        }

        // Drain any results that arrived from the worker.
        loop {
            match res_rx.try_recv() {
                Ok(SearchMsg::Result { query, skills }) => {
                    if state.in_flight_for.as_deref() == Some(query.as_str())
                        && state.query == query
                    {
                        state.results = skills;
                        state.selected = 0;
                        state.loading = false;
                        state.in_flight_for = None;
                        render(out, &mut state)?;
                    }
                }
                Ok(SearchMsg::Failed { query }) => {
                    if state.in_flight_for.as_deref() == Some(query.as_str())
                        && state.query == query
                    {
                        state.results.clear();
                        state.loading = false;
                        state.in_flight_for = None;
                        render(out, &mut state)?;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }
}

fn render(out: &mut impl Write, state: &mut PickerState) -> Result<()> {
    // ANSI: move cursor up `last_lines`, go to col 1, clear-down.
    if state.last_lines > 0 {
        execute!(out, cursor::MoveUp(state.last_lines))?;
    }
    execute!(out, cursor::MoveToColumn(0), Clear(ClearType::FromCursorDown))?;

    let mut lines: Vec<String> = Vec::new();

    // Search input
    let cursor_glyph = format!("{}", style("_").bold());
    lines.push(format!(
        "{} {}{}",
        style("Search skills:").dim(),
        state.query,
        cursor_glyph
    ));
    lines.push(String::new());

    if state.query.is_empty() || state.query.len() < 2 {
        lines.push(format!(
            "{}",
            style("Start typing to search (min 2 chars)").dim()
        ));
    } else if state.results.is_empty() && state.loading {
        lines.push(format!("{}", style("Searching...").dim()));
    } else if state.results.is_empty() {
        lines.push(format!("{}", style("No skills found").dim()));
    } else {
        let visible = state.results.iter().take(8);
        for (i, skill) in visible.enumerate() {
            let is_selected = i == state.selected;
            let arrow = if is_selected {
                format!("{}", style(">").bold())
            } else {
                " ".to_string()
            };
            let pkg = if skill.source.is_empty() {
                &skill.id
            } else {
                &skill.source
            };
            let name_at_pkg = format!("{}@{}", pkg, skill.name);
            let name_styled = if is_selected {
                format!("{}", style(name_at_pkg).bold())
            } else {
                name_at_pkg
            };
            let installs_str = format_installs(skill.installs);
            let installs_part = if installs_str.is_empty() {
                String::new()
            } else {
                format!(" {}", style(installs_str).cyan())
            };
            let loading_indicator = if state.loading && i == 0 {
                format!(" {}", style("...").dim())
            } else {
                String::new()
            };
            lines.push(format!(
                "  {} {}{}{}",
                arrow, name_styled, installs_part, loading_indicator
            ));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "{}",
        style("up/down navigate | enter select | esc cancel").dim()
    ));

    for line in &lines {
        // Raw mode disables LF→CRLF translation; emit CRLF explicitly.
        write!(out, "{}\r\n", line)?;
    }
    out.flush()?;
    state.last_lines = lines.len() as u16;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers

fn format_installs(count: u64) -> String {
    if count == 0 {
        return String::new();
    }
    if count >= 1_000_000 {
        let n = count as f64 / 1_000_000.0;
        let s = format!("{:.1}", n);
        format!("{}M installs", s.trim_end_matches(".0"))
    } else if count >= 1_000 {
        let n = count as f64 / 1_000.0;
        let s = format!("{:.1}", n);
        format!("{}K installs", s.trim_end_matches(".0"))
    } else {
        let suffix = if count == 1 { "install" } else { "installs" };
        format!("{} {}", count, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_installs_zero_is_empty() {
        assert_eq!(format_installs(0), "");
    }

    #[test]
    fn format_installs_one_singular() {
        assert_eq!(format_installs(1), "1 install");
    }

    #[test]
    fn format_installs_small_plural() {
        assert_eq!(format_installs(42), "42 installs");
    }

    #[test]
    fn format_installs_thousands_with_decimal() {
        assert_eq!(format_installs(1_200), "1.2K installs");
    }

    #[test]
    fn format_installs_thousands_round_drops_decimal() {
        assert_eq!(format_installs(2_000), "2K installs");
    }

    #[test]
    fn format_installs_matches_vercel_138_2k() {
        assert_eq!(format_installs(138_200), "138.2K installs");
    }

    #[test]
    fn format_installs_million_round_drops_decimal() {
        assert_eq!(format_installs(1_000_000), "1M installs");
    }

    #[test]
    fn format_installs_million_with_decimal() {
        assert_eq!(format_installs(1_500_000), "1.5M installs");
    }
}
