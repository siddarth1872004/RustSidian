#!/usr/bin/env bash
# Budgets that need a display: cold start to a presented frame, idle RAM, idle CPU, binary size.
# Usage: scripts/window-budget.sh <dir with vault-5000, vault-10000, vault-50000> [rustsidian binary]
# Generate the vaults first with: cargo run --release -p rustsidian --example perf_budget -- --dir <dir> --sizes 5000,10000,50000 --report-only
set -u
DIR=${1:?vault dir}
BIN=${2:-target/release/rustsidian}
fail=0

size=$(stat -c %s "$BIN")
echo "binary size: $((size / 1024)) KiB (budget 15360 KiB)"
[ "$size" -le $((15 * 1024 * 1024)) ] || fail=1

start() { # vault budget_ms
  local worst=0
  for _ in 1 2 3 4 5; do
    timeout 6 "$BIN" "$1" "Note 42" --profile >/dev/null 2>"$DIR/.profile.log"
    ms=$(grep -o 'first frame presented [0-9.]*ms' "$DIR/.profile.log" | grep -o '[0-9.]*' | head -1)
    [ -n "$ms" ] || { echo "no frame presented for $1 (no display?)"; return 2; }
    worst=$(awk -v a="$worst" -v b="$ms" 'BEGIN { print (b > a) ? b : a }')
  done
  echo "cold start to first frame, $(basename "$1"): worst of 5 = ${worst} ms (budget $2 ms)"
  awk -v w="$worst" -v b="$2" 'BEGIN { exit !(w <= b) }' || fail=1
}
start "$DIR/vault-5000" 150
start "$DIR/vault-50000" 400

"$BIN" "$DIR/vault-10000" "Note 42" >/dev/null 2>&1 &
pid=$!
sleep 5
rss=$(awk '/VmRSS/ { print $2 }' /proc/$pid/status)
t0=$(awk '{ print $14 + $15 }' /proc/$pid/stat)
sleep 5
t1=$(awk '{ print $14 + $15 }' /proc/$pid/stat)
kill $pid
echo "idle RSS, 10,000 notes: $((rss / 1024)) MiB (budget 80 MiB)"
[ "$rss" -le $((80 * 1024)) ] || fail=1
echo "idle CPU over 5 s: $((t1 - t0)) ticks (budget 0)"
[ $((t1 - t0)) -eq 0 ] || fail=1
exit $fail
