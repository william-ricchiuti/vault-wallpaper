#!/usr/bin/env bash
# Phase 0 measurement: average CPU % over a window and memory footprint at the end,
# for a process plus any WebKit helper processes it spawned.
#
#   measure.sh <label> <warmup-s> <window-s> -- <command...>
#
# WebKit helpers (WebContent, GPU, Networking) are XPC services owned by launchd, so
# they are found by diffing the helper list before and after launch.
set -euo pipefail
label=$1; warmup=$2; window=$3; shift 4

helpers() { pgrep -f 'com.apple.WebKit.(WebContent|GPU|Networking)' | sort || true; }
cpu_secs() { # cumulative CPU seconds for a pid list, from ps TIME (m:ss.xx or h:mm:ss)
  local total=0
  for p in "$@"; do
    t=$(ps -o time= -p "$p" 2>/dev/null | tr -d ' ') || continue
    [ -z "$t" ] && continue
    total=$(awk -v t="$t" -v acc="$total" 'BEGIN{n=split(t,a,":"); s=0; for(i=1;i<=n;i++) s=s*60+a[i]; print acc+s}')
  done
  echo "$total"
}
footprint_mb() { # phys_footprint summed across pids
  local total=0
  for p in "$@"; do
    kb=$(footprint -p "$p" 2>/dev/null | awk '/Footprint:/ {for(i=1;i<=NF;i++) if($i=="Footprint:"){v=$(i+1); u=$(i+2)}; if (u ~ /^M/) v*=1024; if (u ~ /^G/) v*=1048576; if (u ~ /^B/) v/=1024; print v; exit}') || true
    [ -z "${kb:-}" ] && continue
    total=$(awk -v a="$total" -v b="$kb" 'BEGIN{print a+b}')
  done
  awk -v k="$total" 'BEGIN{printf "%.0f", k/1024}'
}

before=$(helpers)
log=$(mktemp)
"$@" >"$log" 2>&1 &
main=$!
sleep "$warmup"
new=$(comm -13 <(echo "$before") <(helpers) | tr '\n' ' ')
pids="$main $new"
c0=$(cpu_secs $pids)
sleep "$window"
c1=$(cpu_secs $pids)
mem=$(footprint_mb $pids)
cpu=$(awk -v a="$c0" -v b="$c1" -v w="$window" 'BEGIN{printf "%.1f", (b-a)/w*100}')
vis=$(grep -o 'visible=[a-z]*' "$log" | tail -1 || true)
echo "$label | cpu=${cpu}% | mem=${mem}MB | procs=$(echo $pids | wc -w | tr -d ' ') | ${vis:-visible=?}"
kill "$main" 2>/dev/null || true
wait "$main" 2>/dev/null || true
rm -f "$log"
