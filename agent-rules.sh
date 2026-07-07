#!/usr/bin/env bash
# ==============================================================================
# agent-rules.sh — add an l0-compressor "prefix noisy commands" rule to AI agents whose
# hook API cannot rewrite a command (so transparent wrapping is impossible).
#
# Cursor, Cline, Copilot, Codex (and similar) can only allow/deny a shell command,
# not rewrite it — see `agent-hook.sh` for the agents that CAN be wrapped
# transparently (Claude Code, Gemini CLI). For the rest, the practical integration
# is a prompt rule: instruct the model to prefix read-only/noisy commands with
# `l0-compressor` itself. This is best-effort (model-dependent), not a hard hook.
#
# Rules are PROJECT-level, so run this from the repo where you want it active.
#
# Usage:
#   ./agent-rules.sh print              Print the rule snippet (paste anywhere)
#   ./agent-rules.sh install <agent>    Add the rule to the agent's rules file
#   ./agent-rules.sh remove  <agent>    Remove it
#   ./agent-rules.sh status  <agent>    Show whether it is installed
#
#   <agent> ∈ cursor | cline | copilot | codex
#     cursor  → .cursor/rules/l0-compressor.mdc        (dedicated file)
#     cline   → .clinerules/l0-compressor.md           (dedicated file)
#     copilot → .github/copilot-instructions.md   (appended, marked block)
#     codex   → AGENTS.md                          (appended, marked block)
# ==============================================================================
set -euo pipefail

BEGIN_MARK="<!-- l0-compressor:begin -->"
END_MARK="<!-- l0-compressor:end -->"

c_g=''; c_y=''; c_b=''; c_0=''
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  c_g=$'\033[0;32m'; c_y=$'\033[0;33m'; c_b=$'\033[0;34m'; c_0=$'\033[0m'
fi
ok()   { printf '  %s●%s %s\n' "$c_g" "$c_0" "$*"; }
warn() { printf '  %s●%s %s\n' "$c_y" "$c_0" "$*"; }
info() { printf '  %s●%s %s\n' "$c_b" "$c_0" "$*"; }

# The rule body (between the markers).
snippet() {
  cat <<EOF
$BEGIN_MARK
## Save tokens: run noisy commands through l0-compressor

To reduce token usage, prefix **read-only / noisy** shell commands with
\`l0-compressor\` — it filters their output (head/tail + de-duplication) before it
reaches you. Examples: \`l0-compressor cargo test\`, \`l0-compressor git log\`,
\`l0-compressor npm run build\`, \`l0-compressor docker compose logs\`.

Do **not** prefix: interactive programs (vim, ssh, less, psql), stateful builtins
(cd, export, source), or piped/compound commands (\`a | b\`, \`a && b\`) — run
those normally. l0-compressor passes such commands through untouched anyway, but
prefixing them adds no value.
$END_MARK
EOF
}

# Per-agent target file + whether it is a dedicated file or an appended block.
AGENT=''; FILE=''; MODE=''   # MODE: dedicated | append
FRONTMATTER=''
select_agent() {
  AGENT="${1:-}"
  FRONTMATTER=''
  case "$AGENT" in
    cursor)
      FILE=".cursor/rules/l0-compressor.mdc"; MODE="dedicated"
      FRONTMATTER=$'---\ndescription: Prefix noisy shell commands with l0-compressor to save tokens\nalwaysApply: true\n---\n'
      ;;
    cline)   FILE=".clinerules/l0-compressor.md"; MODE="dedicated" ;;
    copilot) FILE=".github/copilot-instructions.md"; MODE="append" ;;
    codex)   FILE="AGENTS.md"; MODE="append" ;;
    "") err_usage "missing agent" ;;
    *) err_usage "unknown agent '$AGENT'" ;;
  esac
}

err_usage() {
  printf '  %s●%s %s\n' "${c_y:-}" "${c_0:-}" "$1" >&2
  echo "  agents: cursor | cline | copilot | codex" >&2
  exit 1
}

strip_block() { # remove the marked block from $1 (in place) + trailing blank lines
  local f=$1
  [ -f "$f" ] || return 0
  # Buffer the non-block lines, then drop trailing blanks so repeated
  # install→remove cycles don't accumulate blank separators.
  awk -v b="$BEGIN_MARK" -v e="$END_MARK" '
    index($0,b){skip=1}
    skip==0{buf[n++]=$0}
    index($0,e){skip=0}
    END{while(n>0 && buf[n-1]=="")n--; for(i=0;i<n;i++)print buf[i]}
  ' "$f" > "$f.l0tmp" && mv "$f.l0tmp" "$f"
}

cmd_install() {
  if [ "$MODE" = "dedicated" ]; then
    mkdir -p "$(dirname "$FILE")"
    { [ -n "$FRONTMATTER" ] && printf '%s' "$FRONTMATTER"; snippet; } > "$FILE"
    ok "Wrote rule: $FILE"
  else
    mkdir -p "$(dirname "$FILE")"
    [ -f "$FILE" ] && strip_block "$FILE"   # idempotent: drop any prior block
    local sep=''
    [ -s "$FILE" ] && sep=$'\n'             # blank line before the block if file is non-empty
    { printf '%s' "$sep"; snippet; } >> "$FILE"
    ok "Added l0-compressor rule block to: $FILE"
  fi
  info "Rule is project-level; commit it to share with your team."
}

cmd_remove() {
  if [ "$MODE" = "dedicated" ]; then
    if [ -f "$FILE" ]; then rm -f "$FILE"; ok "Removed $FILE"; else warn "Nothing to remove ($FILE absent)."; fi
  else
    if [ -f "$FILE" ] && grep -qF "$BEGIN_MARK" "$FILE"; then
      strip_block "$FILE"; ok "Removed the l0-compressor rule block from $FILE"
    else
      warn "No l0-compressor rule block found in $FILE."
    fi
  fi
}

cmd_status() {
  if [ "$MODE" = "dedicated" ]; then
    if [ -f "$FILE" ]; then ok "installed: $FILE"; else warn "not installed ($FILE absent)"; fi
  elif [ -f "$FILE" ] && grep -qF "$BEGIN_MARK" "$FILE"; then
    ok "installed: rule block present in $FILE"
  else
    warn "not installed (no rule block in $FILE)"
  fi
}

usage() { awk 'NR==1{next} /^[^#]/{exit} {sub(/^# ?/,""); if ($0 !~ /^=+$/) print}' "$0"; }

action="${1:-print}"
case "$action" in
  print)            snippet ;;
  install)          select_agent "${2:-}"; cmd_install ;;
  remove|uninstall) select_agent "${2:-}"; cmd_remove ;;
  status)           select_agent "${2:-}"; cmd_status ;;
  help|-h|--help)   usage ;;
  *) err_usage "unknown command '$action'" ;;
esac
