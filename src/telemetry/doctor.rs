//! `l0-compressor --doctor`: installation, PATH, shell, and editor diagnostics.

use super::*;

/// Diagnoses the l0-compressor installation, PATH resolution, shell environment, and active LLM editors.
pub fn run_doctor() {
    let ui = crate::ui::Ui::new();
    println!("{}", ui.box_top("l0-compressor DOCTOR", "health check"));
    println!(
        "{}",
        ui.box_row({
            let mut l = ui.line();
            l.paint(
                "38;5;245",
                "System, shell, telemetry & LLM-editor diagnostics",
            );
            l
        })
    );
    println!("{}", ui.box_bottom());
    println!();

    let mut ok_count = 0;
    let mut warn_count = 0;
    let mut err_count = 0;

    // 1. Binary & PATH check
    println!("{}", ui.section("1. Binary & PATH"));
    match std::env::current_exe() {
        Ok(exe_path) => {
            println!(
                "{}",
                ui.field("Executable", &exe_path.display().to_string())
            );

            // Check if current_exe is in PATH directories
            let path_var = std::env::var("PATH").unwrap_or_default();
            let mut found_in_path = false;
            let mut resolved_path = None;

            if let Some(binary_name) = exe_path.file_name() {
                for dir in std::env::split_paths(&path_var) {
                    let candidate = dir.join(binary_name);
                    if candidate.exists() {
                        found_in_path = true;
                        resolved_path = Some(candidate);
                        break;
                    }
                }
            }

            if found_in_path {
                let resolved = resolved_path.unwrap();
                println!(
                    "{}",
                    ui.field("Resolved in PATH", &resolved.display().to_string())
                );
                println!(
                    "{}",
                    ui.ok("l0-compressor is correctly configured in your PATH.")
                );
                ok_count += 1;
            } else {
                println!(
                    "{}",
                    ui.warn("l0-compressor was not found in your PATH directories.")
                );
                println!("{}", ui.hint("run the installer: ./install.sh --local"));
                warn_count += 1;
            }

            // Check for symlink/alias 't'
            let mut t_found = false;
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join("t");
                if candidate.exists() {
                    t_found = true;
                    println!(
                        "{}",
                        ui.field("Short command 't'", &candidate.display().to_string())
                    );
                    break;
                }
            }
            if t_found {
                println!("{}", ui.ok("Short command 't' is installed and ready."));
                ok_count += 1;
            } else {
                println!("{}", ui.warn("Short command 't' not found in PATH."));
                println!(
                    "{}",
                    ui.hint("create a symlink or alias 't' to speed up typing.")
                );
                warn_count += 1;
            }
        }
        Err(e) => {
            println!(
                "{}",
                ui.err(&format!(
                    "Failed to determine current executable path: {}",
                    e
                ))
            );
            err_count += 1;
        }
    }
    println!();

    // 2. Shell Configuration & Auto-completions
    println!("{}", ui.section("2. Shell Configuration & Completions"));
    if let Ok(shell_var) = std::env::var("SHELL") {
        let shell_name = shell_var.rsplit('/').next().unwrap_or(&shell_var);
        println!("{}", ui.field("Active shell", shell_name));

        let home = std::env::var("HOME").unwrap_or_default();
        let mut config_file = None;
        let mut completions_exist = false;

        match shell_name {
            "zsh" => {
                config_file = Some(PathBuf::from(&home).join(".zshrc"));
                let zfunc = PathBuf::from(&home).join(".zfunc").join("_l0-compressor");
                completions_exist = zfunc.exists();
            }
            "bash" => {
                let bashrc = PathBuf::from(&home).join(".bashrc");
                config_file = Some(if bashrc.exists() {
                    bashrc
                } else {
                    PathBuf::from(&home).join(".bash_profile")
                });
                let bash_comp = PathBuf::from(&home)
                    .join(".local/share/bash-completion/completions/l0-compressor");
                completions_exist = bash_comp.exists();
            }
            "fish" => {
                config_file = Some(PathBuf::from(&home).join(".config/fish/config.fish"));
                let fish_comp =
                    PathBuf::from(&home).join(".config/fish/completions/l0-compressor.fish");
                completions_exist = fish_comp.exists();
            }
            _ => {}
        }

        if let Some(ref path) = config_file {
            if path.exists() {
                println!("{}", ui.field("Profile file", &path.display().to_string()));
                if let Ok(content) = fs::read_to_string(path) {
                    if content.contains("l0-compressor") || content.contains("alias t=") {
                        println!(
                            "{}",
                            ui.ok("Shell profile contains l0-compressor references.")
                        );
                        ok_count += 1;
                    } else {
                        println!(
                            "{}",
                            ui.warn(
                                "Shell profile exists but has no active l0-compressor references."
                            )
                        );
                        warn_count += 1;
                    }
                } else {
                    println!("{}", ui.warn("Shell profile exists but is unreadable."));
                    warn_count += 1;
                }
            } else {
                println!(
                    "{}",
                    ui.warn(&format!("Shell profile not found at {}", path.display()))
                );
                warn_count += 1;
            }
        }

        if completions_exist {
            println!("{}", ui.ok("Shell auto-completions are installed."));
            ok_count += 1;
        } else {
            println!("{}", ui.warn("Shell auto-completions are not installed."));
            println!(
                "{}",
                ui.hint("set them up with the installer: ./install.sh --local")
            );
            warn_count += 1;
        }
    } else {
        println!("{}", ui.warn("SHELL environment variable is not set."));
        warn_count += 1;
    }
    println!();

    // 3. Telemetry & File Permissions
    println!("{}", ui.section("3. Telemetry & Permissions"));
    if let Some(metrics_file) = metrics_path() {
        println!(
            "{}",
            ui.field("Metrics file", &metrics_file.display().to_string())
        );
        if metrics_file.exists() {
            if let Ok(meta) = fs::metadata(&metrics_file) {
                #[cfg(not(unix))]
                let _ = &meta;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = meta.permissions().mode();
                    // Secure as long as no group/other access (0400, 0440-as-owner,
                    // 0600 all qualify); only group/world bits are a concern.
                    let no_group_or_world = (mode & 0o077) == 0;
                    if no_group_or_world {
                        println!(
                            "{}",
                            ui.ok(&format!(
                                "Secure permissions ({:03o}, no group/world access).",
                                mode & 0o777
                            ))
                        );
                        ok_count += 1;
                    } else {
                        println!(
                            "{}",
                            ui.warn(&format!(
                                "Insecure permissions: {:o} (group/world access; expected 0600).",
                                mode & 0o777
                            ))
                        );
                        println!(
                            "{}",
                            ui.hint(&format!("secure it: chmod 600 {}", metrics_file.display()))
                        );
                        warn_count += 1;
                    }
                }
                #[cfg(not(unix))]
                {
                    println!("{}", ui.ok("Metrics file exists and is writable."));
                    ok_count += 1;
                }
            } else {
                println!("{}", ui.err("Metrics file exists but is inaccessible."));
                err_count += 1;
            }

            // Probe the SAME lock protocol the binary uses (flock on unix,
            // mkdir elsewhere) — probing the legacy mkdir path would test a
            // mechanism production no longer exercises.
            let mut probe = FileLock::for_data_file(&metrics_file);
            if probe.lock() {
                println!("{}", ui.ok("Telemetry lock is acquirable."));
                ok_count += 1;
            } else {
                println!(
                    "{}",
                    ui.warn("Telemetry lock is busy or not acquirable (best-effort writes still proceed).")
                );
                warn_count += 1;
            }
        } else {
            println!(
                "{}",
                ui.ok("Telemetry file does not exist yet (created on first run).")
            );
            ok_count += 1;
        }
    } else {
        println!(
            "{}",
            ui.err("Failed to resolve metrics file path (HOME and XDG_DATA_HOME are missing).")
        );
        err_count += 1;
    }
    println!();

    // 4. Active LLM & Terminal Editors Check
    println!("{}", ui.section("4. LLM Editors & Terminal Environment"));
    let mut editor_detected = false;

    if std::env::var("CLAUDE_CODE").is_ok() {
        println!(
            "{}",
            ui.field("Detected editor", &ui.green("Claude Code CLI"))
        );
        editor_detected = true;
    }

    if let Ok(term_prog) = std::env::var("TERM_PROGRAM") {
        println!("{}", ui.field("Terminal program", &term_prog));
        if term_prog == "vscode" || term_prog.contains("vscode") {
            println!(
                "{}",
                ui.field("Detected editor", &ui.green("VS Code Terminal"))
            );
            editor_detected = true;
        } else if term_prog.to_lowercase().contains("cursor") {
            println!(
                "{}",
                ui.field("Detected editor", &ui.green("Cursor AI Terminal"))
            );
            editor_detected = true;
        }
    }

    if (std::env::var("VSCODE_GIT_IPC_HANDLE").is_ok() || std::env::var("VSCODE_PORT").is_ok())
        && !editor_detected
    {
        println!(
            "{}",
            ui.field(
                "Detected editor",
                &ui.green("VS Code/Cursor Backend Terminal")
            )
        );
        editor_detected = true;
    }

    if std::env::var("GEMINI_CLI").is_ok() {
        println!(
            "{}",
            ui.field("Detected editor", &ui.green("Gemini CLI Client"))
        );
        editor_detected = true;
    }

    if editor_detected {
        println!(
            "{}",
            ui.ok("Active LLM terminal detected — l0-compressor will intercept AI subcommands.")
        );
        ok_count += 1;
    } else {
        println!(
            "{}",
            ui.warn("Standard shell environment (no active LLM editor detected).")
        );
        println!(
            "{}",
            ui.hint("ensure your editor terminal inherits the shell PATH setup.")
        );
        warn_count += 1;
    }
    println!();

    // 5. Safety Command Guard Check
    println!("{}", ui.section("5. Safety Command Guard"));
    let guard_active = guard_enabled(false, false);
    if guard_active {
        println!(
            "{}",
            ui.ok("ACTIVE — destructive/exfiltrating commands will be blocked.")
        );
        ok_count += 1;
    } else {
        println!(
            "{}",
            ui.warn("INACTIVE — commands run without safety inspection.")
        );
        warn_count += 1;
    }
    println!();

    // 6. Final Report
    println!("{}", ui.box_top("SUMMARY", ""));
    println!(
        "{}",
        ui.box_row({
            let mut l = ui.line();
            l.paint("32", &format!("✔ {} passed", ok_count))
                .text("    ")
                .paint("33", &format!("⚠ {} warnings", warn_count))
                .text("    ")
                .paint(
                    if err_count > 0 { "31" } else { "38;5;245" },
                    &format!("✗ {} errors", err_count),
                );
            l
        })
    );
    println!("{}", ui.box_bottom());

    if err_count == 0 && warn_count == 0 {
        println!(
            "  {}",
            ui.green("● Your l0-compressor installation is healthy and fully optimized.")
        );
    } else if err_count == 0 {
        println!(
            "  {}",
            ui.yellow("● Configuration is functional, with warning recommendations.")
        );
    } else {
        println!(
            "  {}",
            ui.red("● Installation has critical errors. Please resolve them or reinstall.")
        );
    }
}
