use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

fn get_t_bin() -> PathBuf {
    let mut exe = std::env::current_exe().expect("failed to get current exe");
    // target/debug/deps/integration-...
    exe.pop(); // pop filename
    if exe.file_name().and_then(|s| s.to_str()) == Some("deps") {
        exe.pop(); // pop deps
    }
    exe.push("l0-cache");
    exe
}

fn wait_timeout(mut child: Child, timeout: Duration) -> Result<ExitStatus, String> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    return Err("Timeout exceeded".to_string());
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[test]
fn binary_output_does_not_hang() {
    let t_bin = get_t_bin();
    let child = Command::new(&t_bin)
        .args(["dd", "if=/dev/urandom", "bs=1M", "count=1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn l0-cache");

    // Wait with a 5 second timeout
    let _status = wait_timeout(child, Duration::from_secs(5)).expect("dd command hung!");
}

#[test]
fn exit_code_propagation() {
    let t_bin = get_t_bin();
    let child = Command::new(&t_bin)
        .args(["sh", "-c", "exit 42"])
        .spawn()
        .expect("failed to spawn l0-cache");
    let status = wait_timeout(child, Duration::from_secs(5)).unwrap();
    assert_eq!(status.code(), Some(42));
}

#[test]
fn pipe_to_head() {
    let t_bin = get_t_bin();
    let t_bin_str = t_bin.to_str().expect("valid path");
    let child = Command::new("sh")
        .arg("-c")
        .arg(format!("'{}' seq 1 10000 | head -5", t_bin_str))
        .spawn()
        .expect("failed to spawn shell");
    let status = wait_timeout(child, Duration::from_secs(5)).unwrap();
    assert!(status.success() || status.code() == Some(141));
}

#[test]
fn integration_banner_appears_when_truncated() {
    let t_bin = get_t_bin();
    let output = Command::new(&t_bin)
        .args([
            "--head",
            "5",
            "--tail",
            "5",
            "--threshold",
            "10",
            "seq",
            "1",
            "100",
        ])
        .output()
        .expect("failed to execute l0-cache");
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(stdout_str.contains("[Showing 5 head + 5 tail of 100 lines]"));
}

#[test]
fn integration_banner_does_not_appear_when_not_truncated() {
    let t_bin = get_t_bin();
    let output = Command::new(&t_bin)
        .args([
            "--head",
            "5",
            "--tail",
            "5",
            "--threshold",
            "10",
            "seq",
            "1",
            "5",
        ])
        .output()
        .expect("failed to execute l0-cache");
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout_str.contains("truncated=true"));
}

#[test]
fn integration_auto_flag_accepted() {
    let t_bin = get_t_bin();
    let output = Command::new(&t_bin)
        .args(["--auto", "echo", "hello"])
        .output()
        .expect("failed to execute l0-cache with --auto");
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout_str.trim(), "hello");
}

#[test]
fn test_auto_tuning_success_decay_e2e() {
    let t_bin = get_t_bin();
    let mut temp_dir = std::env::temp_dir();
    let unique_sub = format!(
        "l0-cache-test-success-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    temp_dir.push(unique_sub);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Run 1, 2 & 3: Successes (no decay yet because history has 0, 1, 2 successes respectively)
    for _ in 0..3 {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "seq", "1", "200"])
            .output()
            .unwrap();
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        assert!(stdout_str.contains("[Showing 30 head + 30 tail of 200 lines]"));
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr_str.contains("auto-tuning"));
    }

    // Run 4: 20% decay (history has 3 successes -> optimizing head=24 tail=24)
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "seq", "1", "200"])
            .output()
            .unwrap();
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        assert!(stdout_str.contains("[Showing 24 head + 24 tail of 200 lines]"));
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(stderr_str.contains("consecutive successful runs, optimizing head=24 tail=24"));
    }

    // Run 5: still 24 head + 24 tail (history has 4 successes)
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "seq", "1", "200"])
            .output()
            .unwrap();
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        assert!(stdout_str.contains("[Showing 24 head + 24 tail of 200 lines]"));
    }

    // Run 6: 40% decay (history has 5 successes -> optimizing head=18 tail=18)
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "seq", "1", "200"])
            .output()
            .unwrap();
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        assert!(stdout_str.contains("[Showing 18 head + 18 tail of 200 lines"));
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(stderr_str.contains("consecutive successful runs, optimizing head=18 tail=18"));
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_auto_tuning_failure_backoff_e2e() {
    let t_bin = get_t_bin();
    let mut temp_dir = std::env::temp_dir();
    let unique_sub = format!(
        "l0-cache-test-fail-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    temp_dir.push(unique_sub);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // 1st failure (no warning yet because history is empty)
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "sh", "-c", "seq 1 200; exit 1"])
            .output()
            .unwrap();
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr_str.contains("auto-tuning"));
    }

    // 2nd failure: F=1 -> tail_error scales to 240
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "sh", "-c", "seq 1 200; exit 1"])
            .output()
            .unwrap();
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(stderr_str.contains("1 consecutive failures detected, expanding tail_error to 240"));
    }

    // 3rd failure: F=2 -> tail_error scales to 360
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "sh", "-c", "seq 1 200; exit 1"])
            .output()
            .unwrap();
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(stderr_str.contains("2 consecutive failures detected, expanding tail_error to 360"));
    }

    // Successful run resets failures
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "sh", "-c", "seq 1 2; exit 0"])
            .output()
            .unwrap();
        // Since get_adaptive_params runs before command execution, it will still show the warning for 3 consecutive failures:
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(stderr_str.contains("3 consecutive failures detected, expanding tail_error to 480"));
    }

    // The next execution is clean (0 consecutive failures)
    {
        let output = Command::new(&t_bin)
            .env("XDG_DATA_HOME", &temp_dir)
            .args(["--auto", "sh", "-c", "seq 1 2; exit 0"])
            .output()
            .unwrap();
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr_str.contains("auto-tuning"));
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_quiet_flag_suppresses_warnings() {
    let t_bin = get_t_bin();
    let mut temp_dir = std::env::temp_dir();
    let unique_sub = format!(
        "l0-cache-test-quiet-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    temp_dir.push(unique_sub);
    std::fs::create_dir_all(&temp_dir).unwrap();

    // With --quiet, auto-tuning warnings should be suppressed
    let output = Command::new(&t_bin)
        .env("XDG_DATA_HOME", &temp_dir)
        .args(["--auto", "--quiet", "sh", "-c", "seq 1 200; exit 1"])
        .output()
        .unwrap();
    let stderr_str = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr_str.contains("auto-tuning"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// A unique, isolated `XDG_DATA_HOME` temp dir for metrics-touching tests.
fn temp_xdg(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "l0-cache-it-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn tail_error_shows_more_tail_than_success() {
    // The headline behavior: on a non-zero exit the tail window is the larger
    // error tail; on success it's the small tail. This must hold on STDOUT, not
    // just in a stderr notice. (Regression guard for the old expand_tail no-op.)
    let t = get_t_bin();
    let xdg = temp_xdg("tailerr");

    let fail = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .args(["--no-auto", "sh", "-c", "seq 1 500; exit 1"])
        .output()
        .unwrap();
    let fail_out = String::from_utf8_lossy(&fail.stdout);
    // Error tail is 120 lines → line 400 (well past the 30-line success tail) is shown.
    assert!(
        fail_out.lines().any(|l| l == "400"),
        "error tail should reach ~120 deep (line 400 missing)"
    );
    assert!(fail_out.lines().any(|l| l == "500"));

    let ok = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .args(["--no-auto", "sh", "-c", "seq 1 500; exit 0"])
        .output()
        .unwrap();
    let ok_out = String::from_utf8_lossy(&ok.stdout);
    // Success tail is 30 lines → line 400 must NOT appear, but 500 (last) does.
    assert!(
        !ok_out.lines().any(|l| l == "400"),
        "success tail should be small (line 400 should not appear)"
    );
    assert!(ok_out.lines().any(|l| l == "500"));

    let _ = std::fs::remove_dir_all(&xdg);
}

#[test]
fn raw_mode_keeps_all_lines_and_no_json_truncation() {
    let t = get_t_bin();
    let xdg = temp_xdg("raw");
    let out = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .args(["--raw", "seq", "1", "5000"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.lines().any(|l| l == "1"));
    assert!(s.lines().any(|l| l == "2500"));
    assert!(s.lines().any(|l| l == "5000"));
    assert!(!s.contains("omitted for LLM"), "raw must not truncate");

    // A big single-line JSON payload is kept verbatim in --raw.
    let json = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .args([
            "--raw",
            "sh",
            "-c",
            "printf '{'; for i in $(seq 1 3000); do printf a; done; printf '}\\n'",
        ])
        .output()
        .unwrap();
    let js = String::from_utf8_lossy(&json.stdout);
    assert!(!js.contains("Large JSON Payload Truncated"));
    assert!(js.trim().len() >= 3000);
    let _ = std::fs::remove_dir_all(&xdg);
}

#[test]
fn binary_output_carries_explicit_banner() {
    let t = get_t_bin();
    let xdg = temp_xdg("bin");
    let child = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .args(["sh", "-c", "head -c 200000 /dev/urandom"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let out = child.wait_with_output().expect("wait");
    let s = String::from_utf8_lossy(&out.stdout);
    // Never silently forward the whole 200 KB blob.
    assert!(
        out.stdout.len() < 200_000,
        "binary should not be passed through whole"
    );
    assert!(
        s.contains("binary output detected"),
        "binary output should carry an explicit banner"
    );
    let _ = std::fs::remove_dir_all(&xdg);
}

#[cfg(unix)]
#[test]
fn idle_timeout_kills_hung_command() {
    let t = get_t_bin();
    let xdg = temp_xdg("idle");
    let start = Instant::now();
    let child = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .args(["--idle-timeout", "1", "sh", "-c", "sleep 8 | cat"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let _ = wait_timeout(child, Duration::from_secs(5)).expect("idle-timeout did not fire");
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "watchdog should kill the command within ~1-2s, took {:?}",
        start.elapsed()
    );
    let _ = std::fs::remove_dir_all(&xdg);
}

#[cfg(unix)]
#[test]
fn sigterm_is_forwarded_and_propagated() {
    let t = get_t_bin();
    let xdg = temp_xdg("term");
    let child = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .args(["sh", "-c", "sleep 30"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    // Give it a moment to spawn the child group, then SIGTERM the proxy.
    thread::sleep(Duration::from_millis(500));
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .status();
    let status =
        wait_timeout(child, Duration::from_secs(5)).expect("proxy did not exit after SIGTERM");
    // 128 + SIGTERM(15) = 143
    assert_eq!(
        status.code(),
        Some(143),
        "proxy should propagate SIGTERM as 143"
    );
    let _ = std::fs::remove_dir_all(&xdg);
}

#[test]
fn concurrent_metric_writers_do_not_corrupt_the_log() {
    let t = get_t_bin();
    let xdg = temp_xdg("conc");
    let n = 16;
    let mut kids = Vec::new();
    for i in 0..n {
        kids.push(
            Command::new(&t)
                .env("XDG_DATA_HOME", &xdg)
                .env("L0_CACHE_GUARD", "0")
                .args(["--no-auto", "echo", &format!("run{}", i)])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn"),
        );
    }
    for k in kids {
        let _ = wait_timeout(k, Duration::from_secs(10));
    }
    let metrics = xdg.join("l0-cache").join("metrics.jsonl");
    let content = std::fs::read_to_string(&metrics).expect("metrics file");
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        n,
        "every concurrent run should append exactly one line"
    );
    for l in &lines {
        assert!(
            l.starts_with('{') && l.ends_with('}'),
            "interleaved/corrupted JSONL line: {l}"
        );
    }
    let _ = std::fs::remove_dir_all(&xdg);
}

#[test]
fn stats_renders_with_seeded_metrics() {
    let t = get_t_bin();
    let xdg = temp_xdg("stats");
    let dir = xdg.join("l0-cache");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("metrics.jsonl"),
        "{\"ts\":\"2026-06-04T10:00:00Z\",\"cmd\":\"cargo\",\"tokens_saved\":900,\"tokens_raw\":1000}\n\
         {\"ts\":\"2026-06-04T10:01:00Z\",\"cmd\":\"git\",\"tokens_saved\":100,\"tokens_raw\":400}\n",
    )
    .unwrap();
    let out = Command::new(&t)
        .env("XDG_DATA_HOME", &xdg)
        .env("L0_CACHE_GUARD", "0")
        .arg("--stats")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("cargo"));
    assert!(s.contains("Total Runs"));
    let _ = std::fs::remove_dir_all(&xdg);
}
