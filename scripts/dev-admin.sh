#!/usr/bin/env bash
# Start the API and admin UI with the same browser origin.
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

if [[ ! -d web/admin/node_modules ]]; then
  echo 'Install frontend dependencies first: cd web/admin && npm ci' >&2
  exit 1
fi

local_database=false
if [[ -z ${DATABASE_URL:-} ]]; then
  export DATABASE_URL='postgres://cloud:local-only-password@127.0.0.1:5432/cloud'
  local_database=true
fi

if [[ -z ${AUTH_SECRET:-} ]]; then
  umask 077
  mkdir -p .local-dev
  if [[ ! -s .local-dev/auth-secret ]]; then
    node -e "process.stdout.write(require('node:crypto').randomBytes(32).toString('hex'))" > .local-dev/auth-secret
  fi
  AUTH_SECRET=$(< .local-dev/auth-secret)
  export AUTH_SECRET
fi

export BIND_ADDR=127.0.0.1:3000
export APP_ORIGIN=http://127.0.0.1:5173
export CLOUD_API_URL=http://127.0.0.1:3000

if $local_database; then
  if docker compose version >/dev/null 2>&1; then
    echo 'Starting the local Compose database...'
    docker compose up -d --wait postgres
  else
    echo 'Docker Compose unavailable; starting local pgrust (see docs/LOCAL-PGRUST.md for prerequisites)...'
    cargo run -p pgrust-local -- up
  fi
  echo 'Applying local database migrations...'
  cargo run -p cloud-api -- migrate
else
  echo 'Using the exported DATABASE_URL; apply migrations separately if needed.'
fi

echo "Admin URL: $APP_ORIGIN/app"
echo 'Press Ctrl+C to stop the API and UI. The database remains running.'
cargo run -p cloud-api &
api_pid=$!
(cd web/admin && exec node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 5173 --strictPort) &
ui_pid=$!
cleanup() {
  trap - EXIT INT TERM
  kill "$api_pid" "$ui_pid" 2>/dev/null || true
  wait "$api_pid" "$ui_pid" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
wait -n "$api_pid" "$ui_pid"
