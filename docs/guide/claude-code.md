# Claude Code Integration

`l0-cache` is normally invoked explicitly — you, or your AI assistant, prefix a
command with `l0-cache`. For [Claude Code](https://claude.com/claude-code), the
bundled `claude-hook.sh` removes that step: it installs a
[`PreToolUse`](https://docs.claude.com/en/docs/claude-code/hooks) hook that
transparently routes the *simple* Bash commands Claude Code runs through
`l0-cache`, so the model never has to prefix anything.

The integration is **opt-in and off by default**. It is deliberately the
opposite of an intrusive shell shim: it never rewrites your aliases, never sits
between your shell and your commands, and only ever touches the commands Claude
Code itself runs.

## Install

`claude-hook.sh` ships in the repository root. Installation requires
[`jq`](https://jqlang.github.io/jq/) (used to edit `settings.json` and to parse
the hook payload).

```sh
./claude-hook.sh install     # write the wrapper + register the hook (idempotent)
./claude-hook.sh enable      # turn it ON
```

Then **start a new Claude Code session** so the hook is loaded — Claude Code
reads hooks from `settings.json` at session startup. After that first load, the
`enable`/`disable` toggle takes effect immediately, with no restart.

## Commands

| Command | Effect |
|---|---|
| `install` | Write the wrapper to `~/.claude/hooks/l0-cache-wrapper.sh` and register a `PreToolUse` (matcher `Bash`) hook in `settings.json`. Idempotent; saves a timestamped backup. |
| `enable` / `on` | Create the toggle file `~/.config/l0-cache/hook.enabled`. Instant. |
| `disable` / `off` | Remove the toggle file. Instant. |
| `status` | Show the installed/registered/enabled state and the `l0-cache` version. |
| `uninstall` / `remove` | Remove the hook registration and the wrapper script. |

The hook honors `$CLAUDE_CONFIG_DIR` (default `~/.claude`) and `$XDG_CONFIG_HOME`
(default `~/.config`).

## How it works

On every Bash tool call, Claude Code passes the command to the wrapper as a JSON
payload on stdin. The wrapper decides whether it is safe to wrap, and if so emits
an updated `tool_input` with `l0-cache --quiet` prepended to the command. Only the
`command` field changes; every other field is preserved, and **no
`permissionDecision` is set** — wrapped commands still go through your normal
Claude Code permission flow.

### What gets wrapped

A single, simple program invocation — `ls -la`, `cargo test`, `git log`,
`npm run build`, and the like.

### What is passed through untouched

The wrapper is conservative by design. It does **not** wrap a command when it
contains any of:

| Category | Examples |
|---|---|
| Shell operators | `&&`, `\|\|`, `;`, `\|`, `>`, `<`, `` ` ``, `$(...)`, `&` |
| Multiple lines | any embedded newline |
| Stateful builtins | `cd`, `export`, `source`, `.`, `eval`, `exec`, `set`, `unset`, `alias` |
| Shell constructs | `for`, `while`, `until`, `if`, `case`, `function`, `{ … }`, `( … )` |
| Interactive / TUI / REPL | `vim`, `vi`, `nvim`, `nano`, `emacs`, `less`, `more`, `man`, `htop`, `top`, `btop`, `ssh`, `telnet`, `fzf`, `tmux`, `screen`, `watch`, `python`, `python3`, `node`, `irb`, `psql`, `mysql`, `sqlite3`, `tig`, `lazygit` |
| Already wrapped | commands already starting with `l0-cache` / `t` |

These are wrapped by `l0-cache` only when *you* invoke it explicitly; the hook
leaves them alone because wrapping them automatically could change shell state,
break a pipeline, or capture an interactive program's TTY.

### Fail-safe

If the toggle file is absent, or `l0-cache`/`jq` is not on `PATH`, or `jq` fails
to parse the payload, the wrapper exits `0` with no output and the command runs
exactly as Claude Code intended. The hook can never block a command.

## Verifying it works

`l0-cache` does not store anything to replay — it filters output on the fly. The
only evidence of activity is the metrics log. To confirm the hook is active in
the **current** session, run a command with plenty of output (e.g. `seq 1 200`)
*without* prefixing `l0-cache`: if the hook is live, the output comes back
truncated with an `l0-cache` footer. Then check the savings:

```sh
l0-cache --stats          # aggregated token savings
./claude-hook.sh status   # install / enabled state
```

If a session shows no savings at all, the most common cause is that it was
started **before** the hook was installed or enabled — open a new session.

## Other agents (Gemini CLI)

Transparent wrapping requires a hook that can **rewrite** the command before it
runs. Two agents support that today:

| Agent | Hook event | Can rewrite? |
|---|---|---|
| Claude Code | `PreToolUse` (matcher `Bash`) | ✅ via `updatedInput` |
| Gemini CLI | `BeforeTool` (matcher `run_shell_command`) | ✅ via `hookSpecificOutput.tool_input` |
| Cursor | `beforeShellExecution` | ❌ allow/deny/ask only |

`agent-hook.sh` installs the same conservative, fail-safe wrapper for either
Claude Code or Gemini CLI (it also enables [`--recover`](/reference/#recover)):

```sh
./agent-hook.sh install gemini    # or: install claude   (default)
./agent-hook.sh enable            # shared on/off toggle for all installed agents
./agent-hook.sh status gemini
./agent-hook.sh uninstall gemini
```

It honors `$GEMINI_CONFIG_DIR` (default `~/.gemini`) and edits that agent's
`settings.json`. `claude-hook.sh` remains as a Claude-only convenience — either
script works for Claude Code.

**Cursor and most other agents** expose a `beforeShellExecution`-style hook that
can only *approve or block* a command, not rewrite it, so they cannot be wrapped
transparently. For those, `agent-rules.sh` drops a project rule telling the model
to prefix noisy read-only commands with `l0-cache`:

```sh
./agent-rules.sh install cursor   # or: cline | copilot | codex
./agent-rules.sh print            # just print the snippet to paste anywhere
./agent-rules.sh remove cursor
```

This is **best-effort** (the model may ignore it), not a hard hook — it writes a
project-level rule file (e.g. `.cursor/rules/l0-cache.mdc`,
`.github/copilot-instructions.md`, `AGENTS.md`).

## Uninstall

```sh
./claude-hook.sh disable     # stop wrapping immediately (keeps the install)
./claude-hook.sh uninstall   # remove the registration and wrapper entirely
```

`uninstall` saves a timestamped backup of `settings.json` before editing it.
Restart Claude Code to drop the hook from a running session.
