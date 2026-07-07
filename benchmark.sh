#!/bin/bash
# benchmark.sh — measure l0-compressor token savings on a noisy command
# (5000 identical INFO lines + 100 unique ERRORs, exit 1).
set -u

BIN="./target/release/l0-compressor"
[ -x "$BIN" ] || BIN="./target/debug/l0-compressor"
if [ ! -x "$BIN" ]; then
    echo "l0-compressor binary not found — run 'cargo build --release' first." >&2
    exit 1
fi

# ── Color guard: ANSI only on a TTY with NO_COLOR unset ──────────────────────
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    C_FAINT=$'\033[38;5;238m'; C_DIM=$'\033[38;5;245m'; C_CY=$'\033[1;36m'
    C_GR=$'\033[32m'; C_B=$'\033[1m'; C_0=$'\033[0m'
else
    C_FAINT=''; C_DIM=''; C_CY=''; C_GR=''; C_B=''; C_0=''
fi

rule() { local n=$1 ch=$2 i out=''; for ((i = 0; i < n; i++)); do out+="$ch"; done; printf '%s' "$out"; }
boxtop() { printf '%s┌%s┐%s\n' "$C_FAINT" "$(rule 64 ─)" "$C_0"; }
boxbot() { printf '%s└%s┘%s\n' "$C_FAINT" "$(rule 64 ─)" "$C_0"; }
boxrow() { printf '%s│%s %-62s %s│%s\n' "$C_FAINT" "$C_0" "$1" "$C_FAINT" "$C_0"; }

# tokens ≈ bytes / 4 (l0-compressor's default --token-factor), formatted k/M.
humantok() { awk -v n="$1" 'BEGIN{ if(n>=1000000) printf "~%.1fM", n/1000000; else if(n>=1000) printf "~%.1fk", n/1000; else printf "~%d", n }'; }
humansize() { awk -v n="$1" 'BEGIN{ if(n>=1048576) printf "%.1fM", n/1048576; else if(n>=1024) printf "%.1fK", n/1024; else printf "%dB", n }'; }
reduction() { awk -v a="$1" -v b="$2" 'BEGIN{ if(b<=0){printf "—"} else printf "%.1f%%", (1-a/b)*100 }'; }

# ── Mock noisy tool ──────────────────────────────────────────────────────────
mock=$(mktemp)
# Lines lead with the counter so they don't share a collapsible prefix — this
# exercises l0-compressor's head/tail truncation (the headline feature) rather than
# its identical-line collapser, which would otherwise reduce everything to 2 lines.
cat > "$mock" <<'EOF'
#!/bin/bash
for i in $(seq 1 5000); do
    echo "$i 2024-10-12T10:30:00.123Z [INFO] processed item $((i * 7)) ok in $((i % 90))ms"
done
for i in $(seq 1 100); do
    echo "$i 2024-10-12T10:30:01.000Z [ERROR] NullPointerException at module.js:$((i * 3))"
done
exit 1
EOF
chmod +x "$mock"

work=$(mktemp -d)
export XDG_DATA_HOME="$work/data"   # isolate auto-tuning history

boxtop
boxrow "${C_B}l0-compressor benchmark${C_0}"
boxrow "${C_DIM}5000 INFO + 100 ERROR log lines, exits 1${C_0}"
boxbot
echo

# Run a mode, capture lines/bytes/human-size into globals.
run() { # $1 = output file, $2... = command
    local out=$1; shift
    "$@" > "$out" 2>&1
    R_LINES=$(wc -l < "$out" | tr -d ' ')
    R_BYTES=$(wc -c < "$out" | tr -d ' ')
    R_SIZE=$(humansize "$R_BYTES")
    R_TOK=$((R_BYTES / 4))
}

printf '  %srunning baseline (no l0-compressor)…%s\n' "$C_DIM" "$C_0"
run "$work/native" "$mock"
b_lines=$R_LINES; b_size=$R_SIZE; b_tok=$R_TOK

printf '  %srunning l0-compressor --no-auto…%s\n' "$C_DIM" "$C_0"
run "$work/no_auto" "$BIN" --no-auto "$mock"
n_lines=$R_LINES; n_size=$R_SIZE; n_tok=$R_TOK

printf '  %srunning l0-compressor --auto (3× to trip failure backoff)…%s\n' "$C_DIM" "$C_0"
"$BIN" --auto "$mock" > /dev/null 2>&1
"$BIN" --auto "$mock" > /dev/null 2>&1
run "$work/auto" "$BIN" --auto "$mock"
a_lines=$R_LINES; a_size=$R_SIZE; a_tok=$R_TOK
echo

# ── Results table ────────────────────────────────────────────────────────────
printf '  %s%-22s %8s %9s %11s %11s%s\n' "$C_DIM" "MODE" "LINES" "SIZE" "~TOKENS" "REDUCTION" "$C_0"
printf '  %s%s%s\n' "$C_FAINT" "$(rule 64 ─)" "$C_0"
printf '  %-22s %8s %9s %11s %11s\n' "baseline" "$b_lines" "$b_size" "$(humantok "$b_tok")" "—"
printf '  %-22s %8s %9s %11s %s%11s%s\n' "l0-compressor --no-auto" "$n_lines" "$n_size" "$(humantok "$n_tok")" "$C_GR" "$(reduction "$n_tok" "$b_tok")" "$C_0"
printf '  %-22s %8s %9s %11s %s%11s%s\n' "l0-compressor --auto" "$a_lines" "$a_size" "$(humantok "$a_tok")" "$C_GR" "$(reduction "$a_tok" "$b_tok")" "$C_0"
echo
printf '  %s●%s best reduction: %s--no-auto%s (%s fewer tokens vs baseline)\n' \
    "$C_CY" "$C_0" "$C_B" "$C_0" "$(reduction "$n_tok" "$b_tok")"

# ── Cleanup ──────────────────────────────────────────────────────────────────
rm -rf "$mock" "$work"
