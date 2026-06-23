//! `--stats` / `--discover` dashboard rendering and number formatting.

use crate::telemetry::*;

/// Print aggregated stats from the metrics file.
pub fn print_stats(since: Option<&str>, json: bool, cost_per_mtok: f64) {
    let agg = match aggregate_metrics(since) {
        StatsData::NoDataDir => {
            eprintln!("l0-cache: cannot determine data directory.");
            eprintln!("   $HOME and $XDG_DATA_HOME are not set, and /etc/passwd lookup failed.");
            eprintln!("   Set $HOME or $XDG_DATA_HOME to enable metrics.");
            return;
        }
        StatsData::NoFile(p) => {
            println!("No metrics found at {}", p.display());
            println!("Run some commands with `l0-cache` first.");
            return;
        }
        StatsData::Empty => {
            println!("No metrics found for the specified period.");
            return;
        }
        StatsData::Ready(a) => a,
    };

    if json {
        print_stats_json(&agg, cost_per_mtok);
        return;
    }

    let ui = crate::ui::Ui::new();
    print!("{}", render_stats_text(&agg, &ui, since, cost_per_mtok));
}

/// Format an efficiency percentage. "100.0%" is reserved for a true
/// saved == raw; anything else ≥99.95% floors to 99.9% — `{:.1}` rounding
/// used to fabricate a perfect score for 99.97%-efficient commands.
pub(crate) fn fmt_pct(saved: usize, raw: usize) -> String {
    let p = pct(saved, raw).min(100.0);
    // `!=` and not `<`: tampered records with saved > raw clamp to p == 100.0
    // and would otherwise print the very "100.0%" this guard reserves.
    if saved != raw && p > 99.9 {
        "99.9%".to_string()
    } else {
        format!("{:.1}%", p)
    }
}

/// Build the full text dashboard. Pure with respect to its inputs so the
/// rendering — markers, thresholds, number formats, footers — is unit-testable
/// (it used to println! straight to stdout, shipping every regression silently).
pub(crate) fn render_stats_text(
    agg: &StatsAgg,
    ui: &crate::ui::Ui,
    since: Option<&str>,
    cost_per_mtok: f64,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    let total_runs = agg.total_runs;
    let total_tokens_saved = agg.total_saved;
    let total_tokens_raw = agg.total_raw;
    let avg_pct = pct(total_tokens_saved, total_tokens_raw);
    // The COMMAND column absorbs extra terminal width (10..=24 columns).
    let name_w = 10 + (ui.inner - crate::ui::INNER).min(14);

    let period = match since {
        Some(s) => format!("last {}", s),
        None => "all-time".to_string(),
    };

    // ── Summary card ─────────────────────────────────────────────────────
    let _ = writeln!(out, "{}", ui.box_top("l0-cache TELEMETRY", &period));

    let mut row = ui.line();
    row.paint("38;5;245", "Runs")
        .pad(12)
        .paint("1", &format_number(total_runs));
    let _ = writeln!(out, "{}", ui.box_row(row));

    let mut row = ui.line();
    row.paint("38;5;245", "Saved")
        .pad(12)
        .paint(ui.pct_code(avg_pct), &format_tokens(total_tokens_saved))
        .paint(
            "38;5;238",
            &format!("  of {} raw · est. tokens", format_tokens(total_tokens_raw)),
        );
    let _ = writeln!(out, "{}", ui.box_row(row));

    let (gauge, gw) = crate::ui::meter(ui, avg_pct, 24);
    let mut row = ui.line();
    row.paint("38;5;245", "Efficiency")
        .pad(12)
        .paint(
            ui.pct_code(avg_pct),
            &format!("{:>6}", fmt_pct(total_tokens_saved, total_tokens_raw)),
        )
        .text("  ")
        .raw(&gauge, gw);
    let _ = writeln!(out, "{}", ui.box_row(row));

    // Unweighted median per-run efficiency: the honest companion to the
    // token-weighted gauge above, which one huge command can dominate.
    let mut row = ui.line();
    row.paint("38;5;245", "Median/run")
        .pad(12)
        .paint(
            ui.pct_code(agg.median_run_pct),
            &format!("{:>6}", format!("{:.1}%", agg.median_run_pct)),
        )
        .paint("38;5;238", "  unweighted");
    let _ = writeln!(out, "{}", ui.box_row(row));

    // Dominance disclosure: when one command holds >50% of all savings, the
    // headline gauge is mostly that command's story — say so.
    if let Some((top_cmd, top_stats)) = agg.by_cmd.first() {
        if total_tokens_saved > 0 {
            let share = 100.0 * top_stats.tokens_saved_total as f64 / total_tokens_saved as f64;
            if share > 50.0 {
                let mut row = ui.line();
                row.pad(12).paint(
                    "38;5;238",
                    &format!(
                        "{} accounts for {:.0}% of savings",
                        safe_label(top_cmd, name_w),
                        share
                    ),
                );
                let _ = writeln!(out, "{}", ui.box_row(row));
            }
        }
    }

    if cost_shown(cost_per_mtok) {
        let mut row = ui.line();
        row.paint("38;5;245", "Cost saved")
            .pad(12)
            .paint(
                "32",
                &format!("${:.2}", usd(total_tokens_saved, cost_per_mtok)),
            )
            .paint("38;5;238", &format!("  @ ${:.2}/Mtok", cost_per_mtok));
        let _ = writeln!(out, "{}", ui.box_row(row));
    }

    let _ = writeln!(out, "{}", ui.box_div());

    // ── Per-command table ────────────────────────────────────────────────
    let sorted = &agg.by_cmd; // already sorted by tokens saved (desc)

    let mut hdr = ui.line();
    hdr.paint("38;5;245", &pad_cols("COMMAND", name_w))
        .text(" ")
        .paint("38;5;245", &format!("{:>5}", "RUNS"))
        .text("  ")
        .paint("38;5;245", &format!("{:>6}", "SAVED"))
        .text("  ")
        .paint("38;5;245", &format!("{:>6}", "EFFIC."))
        .text(" ")
        .paint("38;5;245", "IMPACT");
    let _ = writeln!(out, "{}", ui.box_row(hdr));

    for (i, (cmd, stats)) in sorted.iter().enumerate() {
        let eff_pct = pct(stats.tokens_saved_total, stats.tokens_raw_total);

        // Sanitize + clamp to the name column (the metrics file is externally
        // writable: drop control chars and stay char-boundary safe).
        let cmd_disp = safe_label(cmd, name_w);

        // IMPACT = this command's share of all savings (sqrt-scaled so small
        // rows stay visible), colored by efficiency. The bar used to re-plot
        // the same number as the EFFIC. column, hiding skew like one command
        // holding 90% of savings behind a full-looking bar for everyone.
        let share = if agg.total_saved > 0 {
            stats.tokens_saved_total as f64 / agg.total_saved as f64
        } else {
            0.0
        };
        let fill = 100.0 * share.sqrt();
        let (bar, bw) = crate::ui::meter_scaled(ui, fill, eff_pct, 12);
        let low = stats.is_low_value();

        let mut row = ui.line();
        row.paint("38;5;252", &pad_cols(&cmd_disp, name_w))
            .text(" ")
            .paint("38;5;245", &format!("{:>5}", format_number(stats.runs)))
            .text("  ")
            .paint(
                "1",
                &format!("{:>6}", format_tokens(stats.tokens_saved_total)),
            )
            .text("  ")
            .paint(
                ui.pct_code(eff_pct),
                &format!(
                    "{:>6}",
                    fmt_pct(stats.tokens_saved_total, stats.tokens_raw_total)
                ),
            )
            .text(" ")
            .raw(&bar, bw)
            .text("  ");
        // Markers: the actionable warning wins over the celebratory one (the
        // old order suppressed a possible ⚠ on row 0); rows that are red but
        // below the sample-size gate say why they carry no ⚠.
        if low {
            row.paint("33", "⚠ low");
        } else if i == 0 && stats.tokens_saved_total > 0 {
            row.paint("32", "↑ most saved");
        } else if eff_pct < LOW_VALUE_MAX_PCT && stats.runs < LOW_VALUE_MIN_RUNS {
            row.paint("38;5;238", "(n<5)");
        }
        let _ = writeln!(out, "{}", ui.box_row(row));
    }

    // ── Auto-tuning section ──────────────────────────────────────────────
    render_auto_tuning_section(&mut out, ui, agg, name_w);

    let _ = writeln!(out, "{}", ui.box_bottom());

    // ── Footnotes ────────────────────────────────────────────────────────
    let low_savings: Vec<_> = sorted
        .iter()
        .filter(|(_, stats)| stats.is_low_savings())
        .map(|(cmd, _)| (*cmd).clone())
        .collect();

    if !low_savings.is_empty() {
        let _ = writeln!(
            out,
            "  {} low savings on {} — consider dropping the `l0-cache` prefix there",
            ui.yellow("⚠"),
            low_savings.join(", ")
        );
    }

    let zero_output: Vec<_> = sorted
        .iter()
        .filter(|(_, stats)| stats.is_zero_output())
        .map(|(cmd, _)| (*cmd).clone())
        .collect();

    if !zero_output.is_empty() {
        let _ = writeln!(
            out,
            "  {} no output to compress on {} — wrapping is pure overhead, drop the prefix",
            ui.yellow("⚠"),
            zero_output.join(", ")
        );
    }
    let _ = writeln!(
        out,
        "  {} {}",
        ui.dim("metrics"),
        ui.dim(&agg.path.display().to_string())
    );
    out
}

/// Render the Auto-tuning section inside the stats box: total firings, event
/// breakdown, noisy counter, and the top commands by firing count. Honest by
/// design — if the rule never matched, the section says exactly that.
fn render_auto_tuning_section(out: &mut String, ui: &crate::ui::Ui, agg: &StatsAgg, name_w: usize) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "{}", ui.box_div());

    let firings = agg.auto_firings_total();
    let firings_pct = pct(firings, agg.total_runs);

    let mut row = ui.line();
    row.paint("38;5;245", "AUTO-TUNING");
    let _ = writeln!(out, "{}", ui.box_row(row));

    let mut row = ui.line();
    row.paint("38;5;245", "Firings")
        .pad(12)
        .paint("1", &format_number(firings))
        .text("  ")
        .paint(
            "38;5;238",
            &format!(
                "{:.1}% of {} runs",
                firings_pct,
                format_number(agg.total_runs)
            ),
        );
    let _ = writeln!(out, "{}", ui.box_row(row));

    if firings == 0 {
        let mut row = ui.line();
        row.paint(
            "38;5;238",
            "  — no rule matched in this window (auto-tuning quiet)",
        );
        let _ = writeln!(out, "{}", ui.box_row(row));
        return;
    }

    // Per-event breakdown.
    let mut row = ui.line();
    row.text("  ")
        .paint("38;5;245", "expand_tail_err ")
        .paint("1", &format!("{:>4}", format_number(agg.auto_expand_total)))
        .text("   ")
        .paint("38;5;245", "decay_mod ")
        .paint(
            "1",
            &format!("{:>3}", format_number(agg.auto_decay_mod_total)),
        )
        .text("   ")
        .paint("38;5;245", "decay_strong ")
        .paint(
            "1",
            &format!("{:>3}", format_number(agg.auto_decay_strong_total)),
        );
    let _ = writeln!(out, "{}", ui.box_row(row));

    let mut row = ui.line();
    row.text("  ")
        .paint("38;5;245", "proactive_shrink")
        .pad(20)
        .paint(
            "1",
            &format!("{:>4}", format_number(agg.auto_proactive_shrink_total)),
        )
        .text("   ")
        .paint("38;5;245", "decay_steady ")
        .paint(
            "1",
            &format!("{:>3}", format_number(agg.auto_decay_steady_total)),
        )
        .text("   ")
        .paint("38;5;245", "recover ")
        .paint(
            "1",
            &format!("{:>3}", format_number(agg.auto_recover_total)),
        );
    let _ = writeln!(out, "{}", ui.box_row(row));

    // Noisy counter — false-positive expansions (failure-expand on empty
    // output, i.e. classic "no match" exit=1). High noisy% means the rule is
    // burning context on commands whose failures aren't of the kind it helps
    // with; this is the metric the future Step 1 fix is meant to drop to 0.
    let noisy_pct = pct(agg.auto_noisy_total, firings);
    let mut row = ui.line();
    row.text("  ")
        .paint("38;5;245", "noisy")
        .pad(12)
        .paint("1", &format_number(agg.auto_noisy_total))
        .text("   ")
        .paint(
            if agg.auto_noisy_total > 0 {
                "33"
            } else {
                "38;5;238"
            },
            &format!("{:.1}% of firings", noisy_pct),
        );
    if agg.auto_noisy_total > 0 {
        row.text("  ").paint("33", "⚠");
        // Date of the most recent noisy firing: in the all-time view this is
        // how "stale pre-fix history" and "still happening" stay tellable.
        if let Some(ts) = &agg.auto_noisy_last_ts {
            let date = ts.split('T').next().unwrap_or(ts);
            row.text(" ")
                .paint("38;5;238", &format!("last {}", safe_label(date, 12)));
        }
    }
    let _ = writeln!(out, "{}", ui.box_row(row));

    // Top commands by firing count.
    let mut by_firings: Vec<(&String, &CmdStats)> = agg
        .by_cmd
        .iter()
        .filter(|(_, s)| s.auto_firings() > 0)
        .map(|(c, s)| (c, s))
        .collect();
    // Stable input order (by_cmd is already deterministically sorted) plus a
    // name tie-breaker: two commands tied on firings AND on the by_cmd keys
    // would otherwise be ordering-unstable.
    by_firings.sort_by(|(name_a, a), (name_b, b)| {
        b.auto_firings()
            .cmp(&a.auto_firings())
            .then(name_a.cmp(name_b))
    });

    if by_firings.is_empty() {
        return;
    }

    let mut row = ui.line();
    row.paint("38;5;245", "Top cmds (by firings)");
    let _ = writeln!(out, "{}", ui.box_row(row));

    for (cmd, stats) in by_firings.iter().take(3) {
        let cmd_disp = safe_label(cmd, name_w);
        let total = stats.auto_firings();
        let mix = {
            let mut parts: Vec<String> = Vec::new();
            if stats.auto_expand > 0 {
                parts.push(format!("E:{}", stats.auto_expand));
            }
            if stats.auto_decay_mod > 0 {
                parts.push(format!("Dm:{}", stats.auto_decay_mod));
            }
            if stats.auto_decay_strong > 0 {
                parts.push(format!("Ds:{}", stats.auto_decay_strong));
            }
            if stats.auto_proactive_shrink > 0 {
                parts.push(format!("P:{}", stats.auto_proactive_shrink));
            }
            if stats.auto_decay_steady > 0 {
                parts.push(format!("Dsy:{}", stats.auto_decay_steady));
            }
            if stats.auto_recover > 0 {
                parts.push(format!("R:{}", stats.auto_recover));
            }
            parts.join(" ")
        };
        let mut row = ui.line();
        row.text("  ")
            .paint("38;5;252", &pad_cols(&cmd_disp, name_w))
            .text(" ")
            .paint("1", &format!("{:>4}", format_number(total)))
            .text("   ")
            .paint("38;5;245", &mix);
        if stats.auto_noisy > 0 {
            row.text("   ").paint(
                "33",
                &format!("{} noisy ⚠", format_number(stats.auto_noisy)),
            );
        }
        let _ = writeln!(out, "{}", ui.box_row(row));
    }

    // Legend for the per-command mix — the abbreviations were undecipherable
    // without reading the source (and Ds vs Dsy differ by one letter).
    let mut row = ui.line();
    row.text("  ")
        .paint("38;5;238", "E=expand Dm/Ds/Dsy=decay P=shrink R=recover");
    let _ = writeln!(out, "{}", ui.box_row(row));
}

/// Emit the aggregated stats as a single JSON object (for tooling / `--json`).
fn print_stats_json(agg: &StatsAgg, cost_per_mtok: f64) {
    let round1 = |x: f64| (x * 10.0).round() / 10.0;
    let round2 = |x: f64| (x * 100.0).round() / 100.0;

    let commands: Vec<serde_json::Value> = agg
        .by_cmd
        .iter()
        .map(|(cmd, s)| {
            let mut v = serde_json::json!({
                "command": cmd,
                "runs": s.runs,
                "tokens_saved": s.tokens_saved_total,
                "tokens_raw": s.tokens_raw_total,
                "efficiency_pct": round1(pct(s.tokens_saved_total, s.tokens_raw_total)),
                "auto_tuning": {
                    "firings": s.auto_firings(),
                    "expand_tail_err": s.auto_expand,
                    "decay_moderate": s.auto_decay_mod,
                    "decay_strong": s.auto_decay_strong,
                    "proactive_shrink": s.auto_proactive_shrink,
                    "decay_steady": s.auto_decay_steady,
                    "recover_defaults": s.auto_recover,
                    "noisy": s.auto_noisy,
                },
            });
            if cost_shown(cost_per_mtok) {
                v["usd_saved"] =
                    serde_json::json!(round2(usd(s.tokens_saved_total, cost_per_mtok)));
            }
            v
        })
        .collect();

    let firings_total = agg.auto_firings_total();
    let mut out = serde_json::json!({
        "total_runs": agg.total_runs,
        "tokens_saved": agg.total_saved,
        "tokens_raw": agg.total_raw,
        "efficiency_pct": round1(pct(agg.total_saved, agg.total_raw)),
        "median_run_efficiency_pct": round1(agg.median_run_pct),
        "commands": commands,
        "auto_tuning": {
            "firings": firings_total,
            "firings_pct": round1(pct(firings_total, agg.total_runs)),
            "expand_tail_err": agg.auto_expand_total,
            "decay_moderate": agg.auto_decay_mod_total,
            "decay_strong": agg.auto_decay_strong_total,
            "proactive_shrink": agg.auto_proactive_shrink_total,
            "decay_steady": agg.auto_decay_steady_total,
            "recover_defaults": agg.auto_recover_total,
            "noisy": agg.auto_noisy_total,
            "noisy_last_seen": agg.auto_noisy_last_ts,
            "noisy_pct": round1(pct(agg.auto_noisy_total, firings_total)),
        },
    });
    if cost_shown(cost_per_mtok) {
        out["cost_per_mtok"] = serde_json::json!(cost_per_mtok);
        out["usd_saved"] = serde_json::json!(round2(usd(agg.total_saved, cost_per_mtok)));
    }
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
}

/// Print an opinionated optimization advisory derived from the metrics: which
/// prefixed commands are paying off, which to consider dropping, and which carry
/// the biggest raw-token footprint.
pub fn run_discover(since: Option<&str>, cost_per_mtok: f64) {
    let agg = match aggregate_metrics(since) {
        StatsData::Ready(a) => a,
        _ => {
            println!("No metrics yet — run some commands through `l0-cache` first.");
            return;
        }
    };
    let ui = crate::ui::Ui::new();
    let cost = |tokens: usize| -> String {
        if cost_shown(cost_per_mtok) {
            format!("  [${:.2}]", usd(tokens, cost_per_mtok))
        } else {
            String::new()
        }
    };

    println!("{}", ui.bold("l0-cache · optimization advisor"));
    println!();

    // Keep prefixing: meaningful savings, ranked by impact.
    println!("  {} keep prefixing (paying off)", ui.green("●"));
    let keep: Vec<_> = agg
        .by_cmd
        .iter()
        .filter(|(_, s)| {
            s.tokens_saved_total > 0 && pct(s.tokens_saved_total, s.tokens_raw_total) >= 40.0
        })
        .take(6)
        .collect();
    if keep.is_empty() {
        println!("    {}", ui.dim("— nothing with ≥40% savings yet"));
    } else {
        for (cmd, s) in keep {
            println!(
                "    {:<14} {:>4.0}%  {} runs   ~{} saved{}",
                safe_label(cmd, 14),
                pct(s.tokens_saved_total, s.tokens_raw_total).min(100.0),
                s.runs,
                format_tokens(s.tokens_saved_total),
                cost(s.tokens_saved_total),
            );
        }
    }
    println!();

    // Consider dropping: low savings, run often enough to matter.
    println!(
        "  {} consider dropping the prefix (overhead likely exceeds savings)",
        ui.yellow("●")
    );
    // Same predicate as the --stats row marker and footer (single source of
    // truth) — the two surfaces used to disagree on zero-output commands.
    let drop: Vec<_> = agg
        .by_cmd
        .iter()
        .filter(|(_, s)| s.is_low_value())
        .collect();
    if drop.is_empty() {
        println!("    {}", ui.dim("— none"));
    } else {
        for (cmd, s) in drop {
            println!(
                "    {:<14} {:>4.1}%  {} runs",
                safe_label(cmd, 14),
                pct(s.tokens_saved_total, s.tokens_raw_total).min(100.0),
                s.runs
            );
        }
    }
    println!();

    // Biggest footprint: most raw tokens seen (the heavy hitters).
    println!("  {} biggest footprint (most raw tokens)", ui.cyan("●"));
    let mut by_raw: Vec<_> = agg.by_cmd.iter().collect();
    by_raw.sort_by_key(|(_, s)| std::cmp::Reverse(s.tokens_raw_total));
    for (cmd, s) in by_raw.iter().take(3) {
        println!(
            "    {:<14} {} raw   {} runs",
            safe_label(cmd, 14),
            format_tokens(s.tokens_raw_total),
            s.runs
        );
    }
}

/// Unit tiers promote at the value where `{:.1}` rounding would otherwise
/// overflow the previous tier: 999,950+ renders as "1000.0k" (7 chars,
/// breaking the 6-wide table cells), so it must take the M branch instead.
/// Same at the G boundary; with no tier above G the cell stays within 7
/// chars up to ~999.9G tokens (practically unreachable beyond that).
pub(crate) fn format_tokens(n: usize) -> String {
    if n >= 999_950_000 {
        format!("{:.1}G", n as f64 / 1_000_000_000.0)
    } else if n >= 999_950 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

pub(crate) fn format_number(n: usize) -> String {
    if n >= 999_950 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}
