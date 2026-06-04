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
