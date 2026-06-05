#!/usr/bin/env bash
# ==============================================================================
# claude-hook.sh — manage the transparent l0-cache integration for Claude Code.
#
# Installs a PreToolUse hook that routes *simple* Bash commands Claude Code runs
# through `l0-cache` (to cut token usage), without the model prefixing anything.
# It is conservative (compound/piped/interactive/stateful commands pass through
# untouched), fail-safe (any error → the command runs unchanged), and OFF by
# default — toggle it on/off at runtime with no restart.
#
# Usage:
#   ./claude-hook.sh install     Install the wrapper + register the hook (idempotent)
#   ./claude-hook.sh enable      Turn the hook ON  (create the toggle file)
#   ./claude-hook.sh disable     Turn the hook OFF (remove the toggle file)
#   ./claude-hook.sh status      Show install / enabled state + l0-cache version
#   ./claude-hook.sh uninstall   Remove the hook registration and wrapper script
#   ./claude-hook.sh help
#
# Notes:
#   * `install`/`uninstall` edit Claude Code's settings.json and need `jq`.
#   * After install (or after changing settings), start a NEW Claude Code session
#     so the hook is loaded. The enable/disable toggle is then instant.
#   * Honors $CLAUDE_CONFIG_DIR and $XDG_CONFIG_HOME.
# ==============================================================================
set -euo pipefail

CLAUDE_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"
HOOKS_DIR="$CLAUDE_DIR/hooks"
SETTINGS="$CLAUDE_DIR/settings.json"
WRAPPER="$HOOKS_DIR/l0-cache-wrapper.sh"
TOGGLE_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/l0-cache"
TOGGLE="$TOGGLE_DIR/hook.enabled"

# Color only on an interactive terminal with NO_COLOR unset.
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  c_g=$'\033[0;32m'; c_y=$'\033[0;33m'; c_r=$'\033[0;31m'; c_b=$'\033[0;34m'; c_0=$'\033[0m'
else
  c_g=''; c_y=''; c_r=''; c_b=''; c_0=''
fi
info() { printf '  %s●%s %s\n' "$c_b" "$c_0" "$*"; }
ok()   { printf '  %s●%s %s\n' "$c_g" "$c_0" "$*"; }
warn() { printf '  %s●%s %s\n' "$c_y" "$c_0" "$*"; }
err()  { printf '  %s●%s %s\n' "$c_r" "$c_0" "$*" >&2; }

need_jq() {
  command -v jq >/dev/null 2>&1 || { err "jq is required for this command. Install it (e.g. 'brew install jq' / 'apt-get install jq')."; exit 1; }
}

write_wrapper() {
  mkdir -p "$HOOKS_DIR"
  cat > "$WRAPPER" <<'WRAP'
#!/usr/bin/env bash
# Claude Code PreToolUse hook — transparently route simple Bash commands through
# `l0-cache`. CONSERVATIVE + FAIL-SAFE + OFF by default. Managed by claude-hook.sh.
# Toggle:  touch ~/.config/l0-cache/hook.enabled   (on)
#          rm -f ~/.config/l0-cache/hook.enabled   (off)
toggle="${XDG_CONFIG_HOME:-$HOME/.config}/l0-cache/hook.enabled"
[ -f "$toggle" ] || exit 0
command -v l0-cache >/dev/null 2>&1 || exit 0
command -v jq >/dev/null 2>&1 || exit 0

input="$(cat)"
cmd="$(printf '%s' "$input" | jq -r '.tool_input.command // empty' 2>/dev/null)"
[ -n "$cmd" ] || exit 0

# Already wrapped?
case "$cmd" in
  l0-cache\ * | t\ * | */l0-cache\ * | */t\ *) exit 0 ;;
esac

# Risky to wrap a single program: shell operators, redirects, subshells,
# substitution, multi-line, or stateful builtins that must affect the real shell.
case "$cmd" in
  *'&&'* | *'||'* | *';'* | *'|'* | *'>'* | *'<'* | *'`'* | *'$('* | *'&'*) exit 0 ;;
  *$'\n'*) exit 0 ;;
  cd | cd\ * | export\ * | source\ * | .\ * | eval\ * | exec\ * | set\ * | unset\ * | alias\ *) exit 0 ;;
  for\ * | while\ * | until\ * | if\ * | case\ * | function\ * | '{'* | '('*) exit 0 ;;
esac

# Interactive / TUI / REPL programs: l0-cache would capture instead of passthrough.
first="${cmd%% *}"; first="${first##*/}"
case "$first" in
  vim | vi | nvim | nano | emacs | less | more | man | htop | top | btop | ssh | telnet | fzf | tmux | screen | watch | python | python3 | node | irb | psql | mysql | sqlite3 | tig | lazygit) exit 0 ;;
esac

# Wrap. Keep every other tool_input field; only the command changes. No
# permissionDecision → wrapped commands still go through your normal permissions.
printf '%s' "$input" | jq -c \
  --arg new "l0-cache --quiet $cmd" \
  '{hookSpecificOutput: {hookEventName: "PreToolUse", updatedInput: (.tool_input + {command: $new})}}' 2>/dev/null || exit 0
WRAP
  chmod +x "$WRAPPER"
}

cmd_install() {
  need_jq
  info "Installing l0-cache Claude Code hook..."
  write_wrapper
  ok "Wrapper written: $WRAPPER"

  mkdir -p "$CLAUDE_DIR"
  [ -f "$SETTINGS" ] || echo '{}' > "$SETTINGS"
  cp "$SETTINGS" "$SETTINGS.bak.$(date +%s)"

  # Idempotent: drop any prior entry pointing at our wrapper, then append a fresh one.
  local tmp; tmp="$(mktemp)"
  jq --arg h "$WRAPPER" '
    .hooks.PreToolUse = (
      ((.hooks.PreToolUse // []) | map(select((.hooks // []) | any(.command == $h) | not)))
      + [ { matcher: "Bash", hooks: [ { type: "command", command: $h } ] } ]
    )
  ' "$SETTINGS" > "$tmp"
  jq empty "$tmp"            # validate
  mv "$tmp" "$SETTINGS"
  ok "Registered PreToolUse(Bash) hook in $SETTINGS (backup saved)."

  mkdir -p "$TOGGLE_DIR"
  warn "The hook is OFF by default. Enable it with: ./claude-hook.sh enable"
  warn "Then start a NEW Claude Code session so the hook is loaded."
}

cmd_uninstall() {
  need_jq
  [ -f "$SETTINGS" ] || { warn "No settings.json at $SETTINGS — nothing to remove."; }
  if [ -f "$SETTINGS" ]; then
    cp "$SETTINGS" "$SETTINGS.bak.$(date +%s)"
    local tmp; tmp="$(mktemp)"
    jq --arg h "$WRAPPER" '
      (if .hooks.PreToolUse then .hooks.PreToolUse |= map(select((.hooks // []) | any(.command == $h) | not)) else . end)
      | (if (.hooks.PreToolUse // []) == [] then (.hooks |= del(.PreToolUse)) else . end)
      | (if (.hooks // {}) == {} then del(.hooks) else . end)
    ' "$SETTINGS" > "$tmp"
    jq empty "$tmp"
    mv "$tmp" "$SETTINGS"
    ok "Removed the hook registration from $SETTINGS (backup saved)."
  fi
  rm -f "$WRAPPER"; ok "Removed wrapper $WRAPPER"
  rm -f "$TOGGLE"; ok "Toggle cleared (hook OFF)."
  warn "Restart Claude Code to drop the hook from the running session."
}

cmd_enable() {
  mkdir -p "$TOGGLE_DIR"; touch "$TOGGLE"
  ok "Hook ENABLED ($TOGGLE)."
  [ -f "$WRAPPER" ] || warn "Wrapper not installed yet — run: ./claude-hook.sh install"
}

cmd_disable() {
  rm -f "$TOGGLE"
  ok "Hook DISABLED (toggle removed). Takes effect immediately."
}

cmd_status() {
  printf '%sl0-cache Claude Code hook%s\n' "$c_b" "$c_0"
  if command -v l0-cache >/dev/null 2>&1; then ok "l0-cache: $(l0-cache --version 2>/dev/null)"; else err "l0-cache: not found in PATH"; fi
  if [ -f "$WRAPPER" ]; then ok "wrapper installed: $WRAPPER"; else warn "wrapper NOT installed (run: install)"; fi
  if command -v jq >/dev/null 2>&1 && [ -f "$SETTINGS" ] && jq -e --arg h "$WRAPPER" '(.hooks.PreToolUse // []) | any((.hooks // []) | any(.command == $h))' "$SETTINGS" >/dev/null 2>&1; then
    ok "registered in settings.json"
  else
    warn "NOT registered in settings.json (run: install)"
  fi
  if [ -f "$TOGGLE" ]; then ok "state: ENABLED ($TOGGLE)"; else warn "state: DISABLED (run: enable)"; fi
}

usage() { sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; }

case "${1:-help}" in
  install)             cmd_install ;;
  uninstall|remove)    cmd_uninstall ;;
  enable|on)           cmd_enable ;;
  disable|off)         cmd_disable ;;
  status)              cmd_status ;;
  help|-h|--help)      usage ;;
  *) err "Unknown command: ${1:-}"; echo; usage; exit 1 ;;
esac
