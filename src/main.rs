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
    let should_guard = if args.no_guard {
        false
    } else if args.guard {
        true
    } else if let Ok(val) = std::env::var("L0_CACHE_GUARD") {
        val == "1"
    } else {
        telemetry::is_llm_environment()
    };

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
                std::process::exit(127);
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
                let head_cap = head;
                let tail_cap = if result.exit_code == 0 {
                    tail
                } else {
                    tail_error
                };
                let _savings_pct = if result.filter_result.bytes_raw > 0 {
                    (result
                        .filter_result
                        .bytes_raw
                        .saturating_sub(result.filter_result.bytes_final)
                        as f64
                        / result.filter_result.bytes_raw as f64)
                        * 100.0
                } else {
                    0.0
                };
                let separator = if output_to_write.is_empty() || output_to_write.ends_with('\n') {
                    ""
                } else {
                    "\n"
                };
                let banner = format!(
                    "{}\n... [l0-cache: exit_code={}, duration={}ms, truncated=true, {} lines omitted] ...\n... [Showing {} head + {} tail of {} lines] ...\n",
                    separator,
                    result.exit_code,
                    result.duration_ms,
                    result.filter_result.lines_raw.saturating_sub(head_cap + tail_cap),
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
            std::process::exit(127);
        }
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

/// Install signal handlers for clean proxy behavior.
///
/// - SIGINT (Ctrl-C): Ignored in `l0-cache`. The child process receives it from
///   the terminal (same process group). We wait for the child to finish.
/// - SIGTERM: Same treatment — ignore in parent, let child handle it.
/// - SIGPIPE: Ignored so we can handle BrokenPipe in code and still log metrics.
#[cfg(unix)]
fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

#[cfg(not(unix))]
fn install_signal_handlers() {
    // On non-Unix platforms, signals are handled differently.
    // The child process will still receive Ctrl-C via the console.
}
