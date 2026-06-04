//! `l0-cache` — Lightweight CLI proxy that reduces LLM token consumption.
//!
//! Usage:
//!   l0-cache cargo test          # filtered output
//!   l0-cache --raw cargo test    # full output, still logs metrics
//!   l0-cache --stats             # show savings report
//!   l0-cache --stats --since 7d  # last 7 days only
//!   l0-cache -i vim file.txt     # force passthrough (interactive)

mod args;
mod filter;
mod runner;
mod telemetry;

use args::Args;
use clap::Parser;
use std::io::Write;

fn main() {
    let args = Args::parse();

    // ── Shell completions ───────────────────────────────────────────────
    if let Some(shell) = args.completions {
        let mut cmd = <Args as clap::CommandFactory>::command();
        clap_complete::generate(shell, &mut cmd, "l0-cache", &mut std::io::stdout());
        std::process::exit(0);
    }

    // ── Reset stats mode ────────────────────────────────────────────────
    if args.reset_stats {
        if let Err(e) = telemetry::reset_stats() {
            eprintln!("l0-cache: failed to reset stats: {}", e);
            std::process::exit(1);
        }
        println!("l0-cache: telemetry stats have been successfully reset.");
        std::process::exit(0);
    }

    // ── Stats mode ──────────────────────────────────────────────────────
    if args.stats {
        telemetry::print_stats(args.since.as_deref());
        std::process::exit(0);
    }

    // ── Doctor mode ─────────────────────────────────────────────────────
    if args.doctor {
        telemetry::run_doctor();
        std::process::exit(0);
    }

    // ── No command provided ─────────────────────────────────────────────
    if args.command.is_empty() {
        eprintln!("l0-cache: no command specified. Usage: l0-cache <command> [args...]");
        eprintln!("   l0-cache --stats       show token savings report");
        eprintln!("   l0-cache --help        show all options");
        std::process::exit(1);
    }

    // ── Safety Command Guard ────────────────────────────────────────────
    let should_guard = telemetry::guard_enabled(args.guard, args.no_guard);

    if should_guard && !args.command.is_empty() {
        if let Err(reason) = telemetry::check_dangerous_command(&args.cmd_name(), &args.command) {
            eprintln!("\x1b[31m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
            eprintln!(
                "\x1b[31;1m● l0-cache: GUARD BLOCKED A POTENTIALLY DESTRUCTIVE COMMAND\x1b[0m"
            );
            eprintln!("\x1b[31m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
            eprintln!("Reason: {}", reason);
            eprintln!(
                "If this is intentional, bypass the guard using the \x1b[33m--no-guard\x1b[0m flag"
            );
            eprintln!("or set the environment variable \x1b[33mL0_CACHE_GUARD=0\x1b[0m.");
            eprintln!("\x1b[31m━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\x1b[0m");
            std::process::exit(126);
        }
    }

    // ── Signal handling ─────────────────────────────────────────────────
    // SIGINT/SIGTERM: Ignored in parent so the child (same process group)
    // receives them from the terminal. We wait for child.wait() to complete
    // and propagate its exit code. This prevents zombie processes.
    //
    // SIGPIPE: Ignored so `l0-cache cmd | head` doesn't kill us before we can
    // log metrics. We handle BrokenPipe on stdout manually.
    install_signal_handlers();

    // ── Passthrough mode (interactive) ──────────────────────────────────
    if args.should_passthrough() {
        match runner::run_passthrough(&args.command) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("l0-cache: failed to execute '{}': {}", args.cmd_name(), e);
                std::process::exit(exec_error_code(&e));
            }
        }
    }

    // ── Capture mode (default) ──────────────────────────────────────────
    let mut head = args.head;
    let mut tail = args.tail;
    let mut tail_error = args.tail_error;

    if !args.no_auto {
        let tuned = telemetry::get_adaptive_params(
            &args.cmd_name(),
            head,
            tail,
            tail_error,
            args.auto_floor,
            args.auto_ceiling,
        );
        if tuned.modified {
            if let Some(reason) = &tuned.reason {
                if !args.quiet {
                    eprintln!("l0-cache: auto-tuning: {}", reason);
                }
            }
            head = tuned.head;
            tail = tuned.tail;
            tail_error = tuned.tail_error;
        }
    }

    match runner::run_captured(
        &args.command,
        head,
        tail,
        tail_error,
        args.threshold,
        args.raw,
        args.only_errors,
        args.idle_timeout,
    ) {
        Ok(result) => {
            let mut output_to_write = result.filter_result.output.clone();

            if result.filter_result.truncated && result.strategy == "head_tail" {
                // The mid-output "... [N lines omitted for LLM] ..." marker (from the
                // filter) already states the gap; this footer adds only run metadata
                // and the head/tail summary, so the omitted count is not repeated.
                let head_cap = head;
                let tail_cap = if result.exit_code == 0 {
                    tail
                } else {
                    tail_error
                };
                let separator = if output_to_write.is_empty() || output_to_write.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                let banner = format!(
                    "{}\n... [l0-cache: exit_code={}, duration={}ms, truncated=true] ...\n... [Showing {} head + {} tail of {} lines] ...\n",
                    separator,
                    result.exit_code,
                    result.duration_ms,
                    head_cap,
                    tail_cap,
                    result.filter_result.lines_raw
                );
                output_to_write.push_str(&banner);
            }

            let output_result = write_output(&output_to_write);

            // Log metrics BEFORE exiting (even on BrokenPipe)
            let metric = telemetry::ExecutionMetric::from_run_with_factor(
                telemetry::RunMetrics {
                    cmd: &args.cmd_name(),
                    args: &args.cmd_args_string(),
                    bytes_raw: result.filter_result.bytes_raw,
                    bytes_final: result.filter_result.bytes_final,
                    lines_raw: result.filter_result.lines_raw,
                    lines_final: result.filter_result.lines_final,
                    truncated: result.filter_result.truncated,
                    strategy: result.strategy,
                    exit_code: result.exit_code,
                    duration_ms: result.duration_ms,
                },
                args.token_factor,
            );
            telemetry::append_metric(&metric, args.quiet);

            // If stdout was broken (pipe closed), exit cleanly
            if output_result.is_err() {
                std::process::exit(141); // 128 + SIGPIPE(13)
            }

            // Propagate exit code — CRITICAL
            std::process::exit(result.exit_code);
        }
        Err(e) => {
            eprintln!("l0-cache: failed to execute '{}': {}", args.cmd_name(), e);
            std::process::exit(exec_error_code(&e));
        }
    }
}

/// Map a spawn/execution I/O error to a POSIX-flavored exit code:
/// 127 when the command (or `/bin/sh`) was not found, 126 for any other failure
/// to execute it. Reserves 127 for its conventional "not found" meaning.
fn exec_error_code(e: &std::io::Error) -> i32 {
    if e.kind() == std::io::ErrorKind::NotFound {
        127
    } else {
        126
    }
}

/// Write output to stdout, handling BrokenPipe gracefully.
fn write_output(output: &str) -> std::io::Result<()> {
    if output.is_empty() {
        return Ok(());
    }

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();

    handle.write_all(output.as_bytes())?;

    // Ensure trailing newline
    if !output.ends_with('\n') {
        handle.write_all(b"\n")?;
    }

    handle.flush()
}

/// Async-signal-safe handler: forward the received signal to the captured
/// child's process group, so the whole subtree terminates. Only loads an atomic
/// and calls `kill`, both of which are async-signal-safe.
///
/// Because the captured child runs in its OWN process group, the controlling
/// terminal no longer delivers SIGINT directly to it — the parent must forward.
/// This also fixes a directed `kill <l0-cache-pid>` (SIGTERM), which the old
/// `SIG_IGN` swallowed while the child kept running and `child.wait()` blocked.
#[cfg(unix)]
extern "C" fn forward_signal(sig: libc::c_int) {
    let pgid = runner::CHILD_PGID.load(std::sync::atomic::Ordering::SeqCst);
    if pgid > 0 {
        // Negative pid → signal the process group (killpg).
        unsafe {
            libc::kill(-pgid, sig);
        }
    }
    // If no child is running (pgid == 0) we deliberately no-op: l0-cache itself
    // is mid-spawn or finishing up and should not be torn down here.
}

/// Install signal handlers for clean proxy behavior.
///
/// - SIGINT (Ctrl-C) / SIGTERM: forwarded to the child's process group (above).
/// - SIGPIPE: Ignored so we can handle BrokenPipe in code and still log metrics.
#[cfg(unix)]
fn install_signal_handlers() {
    let handler = forward_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

#[cfg(not(unix))]
fn install_signal_handlers() {
    // On non-Unix platforms, signals are handled differently.
    // The child process will still receive Ctrl-C via the console.
}
