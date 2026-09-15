#!/usr/bin/env bash
# User-facing README stills (docs/media/README.md): isolated daemons, no
# login plugin, no desktop config. OpenGL RHI required.
set -euo pipefail
cd "$(dirname "$0")/.."
media="$PWD/docs/media/readme"
mkdir -p "$media" review
scratch=$(mktemp -d /tmp/omastorm-readme.XXXXXX)
mkdir -p "$scratch/runtime" "$scratch/cache" "$scratch/bin"
export XDG_RUNTIME_DIR="$scratch/runtime" XDG_CACHE_HOME="$scratch/cache" TMPDIR="$scratch"
export QT_QPA_PLATFORM=offscreen QT_QPA_PLATFORMTHEME=basic QT_QUICK_BACKEND=rhi QSG_RHI_BACKEND=opengl
unset OMASTORM_RESCAN_PLUGIN OMASTORM_ARCHIVE
jq -r '.sites[] | select(.id=="KTLX") | "center_lat = \(.lat)\ncenter_lon = \(.lon)\nlocked_radar = \"KTLX\""' engine/data/sites.json > "$scratch/ktlx.toml"
: > "$scratch/none.toml"
: > "$scratch/empty-state.json"
cat > "$scratch/bin/curl" <<'EOF'
#!/bin/bash
exit 22
EOF
chmod +x "$scratch/bin/curl"

bash scripts/cargo.sh build --offline --locked --quiet
readme_runtime=$XDG_RUNTIME_DIR
cleanup() { XDG_RUNTIME_DIR="$readme_runtime" target/debug/omastorm-engine stop >/dev/null 2>&1 || true; }
trap cleanup EXIT

wait_ipc() {
  local pid=$1 i
  for i in {1..120}; do
    quickshell ipc --pid "$pid" call keys status >/dev/null 2>&1 && return 0
    sleep .1
  done
  echo "keys IPC never answered (pid $pid)" >&2
  return 1
}

grab() {
  # name delay_ms env... -- ipc steps...
  local name=$1 delay=$2 pid
  shift 2
  local env_args=() steps=()
  while [[ $# -gt 0 ]]; do
    if [[ $1 == -- ]]; then shift; steps=("$@"); break; fi
    env_args+=("$1"); shift
  done
  local path="$media/$name.png"
  rm -f "$path"
  local extra=() a
  local has_config=0
  for a in "${env_args[@]+"${env_args[@]}"}"; do
    [[ $a == OMASTORM_CONFIG=* ]] && has_config=1
  done
  (( has_config )) || extra+=(OMASTORM_CONFIG="$scratch/ktlx.toml")
  env "${env_args[@]+"${env_args[@]}"}" "${extra[@]}" \
    OMASTORM_WIDTH=640 OMASTORM_HEIGHT=480 \
    OMASTORM_CAPTURE_DELAY="$delay" OMASTORM_CAPTURE="$path" \
    bash run.sh > "$scratch/$name.log" 2>&1 &
  pid=$!
  wait_ipc "$pid" || { cat "$scratch/$name.log"; kill "$pid" 2>/dev/null || true; return 1; }
  local step words
  for step in "${steps[@]+"${steps[@]}"}"; do
    read -ra words <<< "$step"
    case ${words[0]} in
      sleep) sleep "${words[1]}" ;;
      wait-matches)
        local needle=${words[1]} m i
        for i in {1..50}; do
          m=$(quickshell ipc --pid "$pid" call location matches 2>/dev/null || true)
          [[ $m == *"$needle"* ]] && break
          sleep .1
        done
        ;;
      locate)
        quickshell ipc --pid "$pid" call keys run locate
        ;;
      location)
        quickshell ipc --pid "$pid" call location "${words[1]}" "${step#location ${words[1]} }"
        ;;
      *) quickshell ipc --pid "$pid" call "${words[@]}" ;;
    esac
  done
  wait "$pid" || true
  [[ -s $path ]] || { cat "$scratch/$name.log"; echo "No capture for $name" >&2; exit 1; }
  echo "captured $name"
}

# Hero and treatments need a live frame; give the poller time.
grab window 15000
grab pixels 8000 OMASTORM_STYLE=PIXELS
grab glyphs 8000 OMASTORM_STYLE=GLYPHS
grab stipple 8000 OMASTORM_STYLE=STIPPLE
magick montage -label '%t' "$media/pixels.png" "$media/glyphs.png" "$media/stipple.png" \
  -tile 3x -geometry 320x240+6+10 -background '#181414' -fill '#e6d9db' -pointsize 14 \
  "$media/treatments.png"
rm -f "$media/pixels.png" "$media/glyphs.png" "$media/stipple.png"
echo "captured treatments"

# First-run prompt: its own daemon, so a previous KTLX select is not on screen.
(
  export XDG_RUNTIME_DIR="$scratch/onboard" XDG_CACHE_HOME="$scratch/onboard-cache" TMPDIR="$scratch/onboard-tmp"
  mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CACHE_HOME" "$TMPDIR"
  grab onboard 5000 \
    OMASTORM_CONFIG="$scratch/none.toml" \
    OMASTORM_STATE="$scratch/empty-state.json" \
    OMASTORM_LOCATION=/dev/null
  target/debug/omastorm-engine stop >/dev/null 2>&1 || true
)

grab search-city 9000 -- 'sleep 3' 'location open tulsa' 'wait-matches Tulsa'
grab search-site 7000 -- 'sleep 3' 'location open ktlx'
grab search-coords 8000 -- 'sleep 3' 'location open 35.4, -97.5'
grab search-error 8000 -- 'sleep 3' 'location open 95, -97.5'

# Same live window; stub curl so the overlay is the lookup failure, not a hang.
PATH="$scratch/bin:$PATH" OMASTORM_LOCATION_URL='http://127.0.0.1:1/nope' \
  grab locate-fail 8000 -- 'sleep 4' locate

# Popover is its own isolated daemon (scripts/capture-popover.sh).
(
  unset XDG_RUNTIME_DIR XDG_CACHE_HOME TMPDIR OMASTORM_CONFIG OMASTORM_STATE OMASTORM_LOCATION
  bash scripts/capture-popover.sh
)
cp review/popover-glyphs.png "$media/popover.png"
echo "captured popover"

echo "$media"
