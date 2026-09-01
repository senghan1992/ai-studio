#!/usr/bin/env bash
# Restart the AI Studio server in the background and wait until the new process
# is the one answering.
#
# The process to stop is found by *listening port*, never by matching its path in
# the process table: a `pgrep -f <path>` also matches any shell or editor whose
# own command line happens to contain that path, and killing those takes down the
# caller instead of the server.
set -u
cd "$(dirname "$0")/.."

LOG="${AI_STUDIO_LOG:-/tmp/ai-studio-server.log}"
PORT="${PORT:-5177}"

pids_on_port() {
  if command -v ss >/dev/null 2>&1; then
    ss -ltnpH 2>/dev/null | awk -v port=":${PORT}$" '
      $4 ~ port { if (match($0, /pid=[0-9]+/)) print substr($0, RSTART+4, RLENGTH-4) }
    ' | sort -u
  elif command -v lsof >/dev/null 2>&1; then
    lsof -ti ":${PORT}" -sTCP:LISTEN 2>/dev/null | sort -u
  fi
}

stop() {
  local pids
  pids=$(pids_on_port)
  [ -z "$pids" ] && return 0
  echo "stopping pid(s) on ${PORT}: $(echo "$pids" | tr '\n' ' ')"
  echo "$pids" | xargs -r kill 2>/dev/null || true
  for _ in $(seq 1 20); do
    [ -z "$(pids_on_port)" ] && return 0
    sleep 0.25
  done
  echo "$(pids_on_port)" | xargs -r kill -9 2>/dev/null || true
  sleep 0.5
}

stop

if [ -n "$(pids_on_port)" ]; then
  echo "port ${PORT} is still held; refusing to start a second server" >&2
  exit 1
fi

nohup node ./apps/server/src/index.js >"$LOG" 2>&1 &
new_pid=$!
disown 2>/dev/null || true

for _ in $(seq 1 60); do
  if ! kill -0 "$new_pid" 2>/dev/null; then
    echo "server exited during startup; log tail:" >&2
    tail -20 "$LOG" >&2
    exit 1
  fi
  if curl -sf "http://localhost:${PORT}/api/health" >/dev/null 2>&1; then
    echo "server ready on ${PORT} (pid ${new_pid}, log: ${LOG})"
    exit 0
  fi
  sleep 0.25
done

echo "server did not answer in time; log tail:" >&2
tail -20 "$LOG" >&2
exit 1
