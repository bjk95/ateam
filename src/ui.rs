use console::{style, Term};
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static VERBOSE: AtomicBool = AtomicBool::new(false);
static QUIET: AtomicBool = AtomicBool::new(false);

pub fn set_verbose(v: bool) {
    VERBOSE.store(v, Ordering::Relaxed);
}

pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

pub fn set_quiet(v: bool) {
    QUIET.store(v, Ordering::Relaxed);
}

pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

pub fn ok(msg: impl AsRef<str>) {
    if is_quiet() {
        return;
    }
    println!("{}", format_ok(msg.as_ref()));
}

pub fn fail(msg: impl AsRef<str>) {
    eprintln!("{}", format_fail(msg.as_ref()));
}

pub fn warn(msg: impl AsRef<str>) {
    eprintln!("{}", format_warn(msg.as_ref()));
}

pub fn detail(msg: impl AsRef<str>) {
    if is_quiet() {
        return;
    }
    if is_verbose() {
        println!("{}", format_detail(msg.as_ref()));
    }
}

pub fn plain(msg: impl AsRef<str>) {
    if is_quiet() {
        return;
    }
    println!("{}", msg.as_ref());
}

pub fn write(msg: impl AsRef<str>) {
    if is_quiet() {
        return;
    }
    print!("{}", msg.as_ref());
}

/// Vercel-skills-CLI-style step marker: cyan ◇ + message.
pub fn diamond(msg: impl AsRef<str>) {
    if is_quiet() {
        return;
    }
    println!("{}", format_diamond(msg.as_ref()));
}

pub fn step(msg: impl Into<String>) -> Step {
    let msg = msg.into();
    if is_quiet() {
        return Step {
            inner: StepImpl::Static,
        };
    }
    if Term::stdout().is_term() {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template("{spinner:.cyan}  {msg}")
                .expect("static template")
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
        );
        pb.set_message(msg);
        pb.enable_steady_tick(Duration::from_millis(80));
        Step {
            inner: StepImpl::Spinner(pb),
        }
    } else {
        println!("{}  {}", style("→").cyan(), msg);
        Step {
            inner: StepImpl::Static,
        }
    }
}

pub struct Step {
    inner: StepImpl,
}

enum StepImpl {
    Spinner(ProgressBar),
    Static,
}

impl Step {
    pub fn ok(self, msg: impl AsRef<str>) {
        self.clear();
        ok(msg);
    }

    pub fn fail(self, msg: impl AsRef<str>) {
        self.clear();
        fail(msg);
    }

    pub fn finish(self) {
        self.clear();
    }

    pub fn set_msg(&self, msg: impl Into<String>) {
        if let StepImpl::Spinner(pb) = &self.inner {
            pb.set_message(msg.into());
        }
    }

    fn clear(&self) {
        if let StepImpl::Spinner(pb) = &self.inner {
            pb.finish_and_clear();
        }
    }
}

impl Drop for Step {
    fn drop(&mut self) {
        // Idempotent in indicatif — safe even if .ok()/.fail()/.finish() already ran.
        self.clear();
    }
}

// ---------------------------------------------------------------------------
// Format-only helpers (testable without capturing stdout).

fn format_ok(msg: &str) -> String {
    format!("{}  {}", style("✓").green(), msg)
}

fn format_fail(msg: &str) -> String {
    format!("{}  {}", style("✗").red(), msg)
}

fn format_warn(msg: &str) -> String {
    format!("{}  {}", style("⚠").yellow(), msg)
}

fn format_detail(msg: &str) -> String {
    format!("{}  {}", style("·").dim(), style(msg).dim())
}

fn format_diamond(msg: &str) -> String {
    format!("{}  {}", style("◇").cyan(), msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init() {
        INIT.call_once(|| {
            console::set_colors_enabled(false);
            console::set_colors_enabled_stderr(false);
        });
    }

    #[test]
    fn ok_format_no_color() {
        init();
        assert_eq!(format_ok("installed foo"), "✓  installed foo");
    }

    #[test]
    fn fail_format_no_color() {
        init();
        assert_eq!(
            format_fail("install bar — not found"),
            "✗  install bar — not found"
        );
    }

    #[test]
    fn warn_format_no_color() {
        init();
        assert_eq!(
            format_warn("unregistered project: foo"),
            "⚠  unregistered project: foo"
        );
    }

    #[test]
    fn detail_format_no_color() {
        init();
        assert_eq!(
            format_detail("source: github:vercel-labs/agent-skills"),
            "·  source: github:vercel-labs/agent-skills"
        );
    }

    #[test]
    fn diamond_format_no_color() {
        init();
        assert_eq!(format_diamond("Source: foo/bar"), "◇  Source: foo/bar");
    }

    #[test]
    fn detail_no_op_when_not_verbose() {
        init();
        set_verbose(false);
        assert!(!is_verbose());
        // Calling detail() shouldn't panic — it's a no-op print path.
        detail("should not appear");
    }

    #[test]
    fn verbose_flag_round_trips() {
        init();
        set_verbose(true);
        assert!(is_verbose());
        set_verbose(false);
        assert!(!is_verbose());
    }

    #[test]
    fn quiet_flag_round_trips() {
        init();
        set_quiet(true);
        assert!(is_quiet());
        set_quiet(false);
        assert!(!is_quiet());
    }
}
