use fs2::FileExt;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn apply_prints_wait_message_when_repo_lock_is_held() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let config_home = tmp.path().join("config");
    let cache_home = tmp.path().join("cache");
    let home = tmp.path().join("home");

    fs::create_dir_all(repo.join(".agents")).unwrap();
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(cache_home.join("agents")).unwrap();
    fs::create_dir_all(&home).unwrap();

    fs::write(repo.join("agents.toml"), "enabled_harnesses = []\n").unwrap();
    fs::write(
        config_home.join("agents.toml"),
        format!("repo = \"{}\"\n", repo.display()),
    )
    .unwrap();
    fs::write(cache_home.join("agents").join("update-check"), "fresh").unwrap();

    let lock_path = repo.join(".agents").join("lock");
    let held_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    held_lock.lock_exclusive().unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_agents"))
        .args(["--no-sync", "apply"])
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CACHE_HOME", &cache_home)
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let result = reader.read_line(&mut line).map(|_| line);
        let _ = tx.send(result);
        let mut rest = String::new();
        let _ = reader.read_to_string(&mut rest);
    });
    let (stderr_tx, stderr_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut output = String::new();
        let result = stderr.read_to_string(&mut output).map(|_| output);
        let _ = stderr_tx.send(result);
    });

    let first_stdout_line = match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(line)) => line,
        Ok(Err(err)) => {
            let _ = child.kill();
            panic!("failed reading apply stdout: {err}");
        }
        Err(_) => {
            let _ = child.kill();
            panic!("apply did not print a repo-lock wait message before blocking");
        }
    };

    assert!(
        first_stdout_line.contains("waiting for another agents process to release repo lock"),
        "unexpected first stdout line: {first_stdout_line:?}"
    );

    drop(held_lock);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let stderr = stderr_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap();
            assert!(
                status.success(),
                "apply exited with {status}\nstderr:\n{stderr}"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("apply did not exit after the repo lock was released");
        }
        thread::sleep(Duration::from_millis(25));
    }
}
