#!/usr/bin/env bash
# Compare riftpdf vs zathura vs Preview on the same PDF.
set -euo pipefail

PDF="${1:-}"
if [[ -z "$PDF" || ! -f "$PDF" ]]; then
  PDF="$(find "$HOME/Desktop/dev/dev_books" -name '*.pdf' -print -quit | head -1)"
fi
if [[ ! -f "$PDF" ]]; then
  echo "usage: $0 /path/to/file.pdf" >&2
  exit 1
fi

RIFTPDF="${RIFTPDF:-$HOME/Desktop/dev/riftpdf/target/release/riftpdf}"
ZATHURA="${ZATHURA:-$(command -v zathura)}"
PREVIEW_BIN="/System/Applications/Preview.app/Contents/MacOS/Preview"
[[ -x "$PREVIEW_BIN" ]] || PREVIEW_BIN="/Applications/Preview.app/Contents/MacOS/Preview"

# dbus for zathura synctex (harmless if unused)
if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]]; then
  _sock="$(launchctl getenv DBUS_LAUNCHD_SESSION_BUS_SOCKET 2>/dev/null || true)"
  [[ -n "$_sock" ]] && export DBUS_SESSION_BUS_ADDRESS="unix:path=$_sock"
fi

kill_matching() {
  local pat="$1"
  pkill -f "$pat" 2>/dev/null || true
  sleep 0.4
}

rss_kb() {
  local pid="$1"
  ps -o rss= -p "$pid" 2>/dev/null | tr -d ' '
}

wait_pid() {
  local pid="$1" max="${2:-40}"
  local i=0
  while kill -0 "$pid" 2>/dev/null; do
    ((i++)) || true
    (( i >= max )) && break
    sleep 0.05
  done
}

echo "============================================"
echo " PDF: $PDF"
echo " size: $(du -h "$PDF" | awk '{print $1}')"
echo " pages (pdfinfo/mutool if available):"
if command -v mutool >/dev/null; then
  mutool info "$PDF" 2>/dev/null | rg -i '^Pages:' || true
elif command -v pdfinfo >/dev/null; then
  pdfinfo "$PDF" 2>/dev/null | rg -i '^Pages:' || true
fi
echo "============================================"
echo

echo "--- Binary / app footprint ---"
printf "riftpdf:  %s\n" "$(ls -lh "$RIFTPDF" | awk '{print $5}')"
printf "zathura:  %s (+ GTK/MuPDF shared libs)\n" "$(ls -lh "$ZATHURA" | awk '{print $5}')"
printf "Preview:  system app (shared frameworks)\n"
echo

# --- Cold start: time until process is alive and has grown past a threshold ---
# GUI first-paint isn't instrumentable without AX hooks; we measure:
#   t_spawn  = time to process start
#   t_ready  = time until RSS exceeds threshold (proxy for document mapped/rendered)

measure_open() {
  local name="$1"
  local threshold_kb="$2"
  shift 2
  # remaining: command...

  kill_matching "$name" >/dev/null 2>&1 || true
  # also kill by binary name hints
  case "$name" in
    riftpdf) pkill -x riftpdf 2>/dev/null || true ;;
    zathura) pkill -x zathura 2>/dev/null || true ;;
    Preview) osascript -e 'tell application "Preview" to quit' 2>/dev/null || true ;;
  esac
  sleep 0.6

  local start end pid rss t_ready=""
  start=$(python3 -c 'import time; print(time.perf_counter())')
  "$@" >/dev/null 2>&1 &
  pid=$!
  # Preview via `open` returns immediately — find real PID
  if [[ "$name" == "Preview" ]]; then
    sleep 0.2
    pid="$(pgrep -n Preview || true)"
  fi

  local deadline
  deadline=$(python3 -c 'import time; print(time.perf_counter()+8)')
  while true; do
    now=$(python3 -c 'import time; print(time.perf_counter())')
    python3 -c "import sys; sys.exit(0 if float('$now')<float('$deadline') else 1)" || break
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      rss="$(rss_kb "$pid" || echo 0)"
      if [[ -n "$rss" && "$rss" -ge "$threshold_kb" ]]; then
        t_ready=$(python3 -c "print(f'{$now - $start:.3f}')")
        break
      fi
    else
      # process may have been replaced (Preview)
      if [[ "$name" == "Preview" ]]; then
        pid="$(pgrep -n Preview || true)"
      fi
    fi
    sleep 0.03
  done
  end=$(python3 -c 'import time; print(time.perf_counter())')
  # settle memory
  sleep 2.5
  if [[ "$name" == "Preview" ]]; then
    pid="$(pgrep -n Preview || true)"
  fi
  rss="$(rss_kb "$pid" || echo 0)"
  local rss_mb
  rss_mb=$(python3 -c "print(f'{int('$rss' or 0)/1024:.1f}')")

  local total
  total=$(python3 -c "print(f'{$end - $start:.3f}')")
  printf "%-10s  ready≈%ss (RSS≥%dMB)   settled RSS=%s MB   pid=%s\n" \
    "$name" "${t_ready:-timeout}" "$((threshold_kb/1024))" "$rss_mb" "${pid:-?}"

  # leave running for side-by-side optional; caller kills
  echo "$pid|$rss_mb|${t_ready:-nan}"
}

echo "--- Cold open (proxy: time until RSS suggests document loaded) ---"
echo "(lower ready time ≈ snappier perceived start; RSS = memory after ~2.5s settle)"
echo

# Thresholds tuned to clear baseline empty-window RSS on this machine
R1=$(measure_open riftpdf 40000 "$RIFTPDF" "$PDF")
kill_matching riftpdf; pkill -x riftpdf 2>/dev/null || true
sleep 0.5

R2=$(measure_open zathura 60000 "$ZATHURA" "$PDF")
kill_matching zathura; pkill -x zathura 2>/dev/null || true
sleep 0.5

R3=$(measure_open Preview 80000 open -a Preview "$PDF")
osascript -e 'tell application "Preview" to quit' 2>/dev/null || true
sleep 0.5

echo
echo "--- Process spawn only (hyperfine, no GUI settle) ---"
if command -v hyperfine >/dev/null; then
  # Preview/open returns instantly; compare CLI viewers only + Preview open latency
  hyperfine --warmup 1 --runs 5 \
    --command-name riftpdf-spawn "($RIFTPDF \"$PDF\" & pid=\$!; sleep 0.15; kill \$pid 2>/dev/null)" \
    --command-name zathura-spawn "($ZATHURA \"$PDF\" & pid=\$!; sleep 0.15; kill \$pid 2>/dev/null)" \
    --command-name preview-open "open -a Preview \"$PDF\"; sleep 0.15; osascript -e 'tell application \"Preview\" to quit' >/dev/null 2>&1" \
    2>&1 | tail -20
  osascript -e 'tell application "Preview" to quit' 2>/dev/null || true
  pkill -x riftpdf 2>/dev/null || true
  pkill -x zathura 2>/dev/null || true
else
  echo "hyperfine not installed"
fi

echo
echo "--- Raw MuPDF page render (riftpdf engine baseline via mutool) ---"
if command -v mutool >/dev/null; then
  hyperfine --warmup 1 --runs 5 \
    --command-name 'mutool draw page1@150dpi' \
    "mutool draw -q -r 150 -o /tmp/rift-bench-p1.png \"$PDF\" 1" \
    --command-name 'mutool draw page110@150dpi' \
    "mutool draw -q -r 150 -o /tmp/rift-bench-p110.png \"$PDF\" 110" \
    2>&1 | tail -15
fi

echo
echo "Done."
