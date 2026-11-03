#!/usr/bin/env bash
# Create a disposable administrator in the default local development database.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

export DATABASE_URL='postgres://cloud:local-only-password@127.0.0.1:5432/cloud'
export APP_ORIGIN='http://127.0.0.1:5173'
if [[ -z ${AUTH_SECRET:-} ]]; then
  if [[ ! -s .local-dev/auth-secret ]]; then
    echo 'Start ./scripts/dev-admin.sh first to initialize the local secret and services.' >&2
    exit 1
  fi
  export AUTH_SECRET=$(< .local-dev/auth-secret)
fi

umask 077
mkdir -p .local-dev
suffix=$(node -e "process.stdout.write(require('node:crypto').randomBytes(8).toString('hex'))")
email="admin-$suffix@example.test"
password=$(node -e "process.stdout.write(require('node:crypto').randomBytes(24).toString('base64url'))")
credentials=.local-dev/test-admin-"$suffix".txt
printf 'Email: %s\nPassword: %s\n' "$email" "$password" > "$credentials"
response=$(mktemp .local-dev/signup-XXXXXXXX)
trap 'rm -f -- "$response"' EXIT

printf '{"name":"Local Test Admin","email":"%s","password":"%s"}' "$email" "$password" |
  curl -fsS -o "$response" -X POST "$APP_ORIGIN/api/auth/sign-up/email" \
    -H "Origin: $APP_ORIGIN" -H 'Content-Type: application/json' --data-binary @- || {
      echo 'Sign-up failed. Ensure ./scripts/dev-admin.sh is running with its default local database.' >&2
      exit 1
    }

# Look up the generated email in the intended local database before granting a role.
user_id=$(psql -X -At -v ON_ERROR_STOP=1 "$DATABASE_URL" -c "SELECT id FROM \"user\" WHERE email = '$email'")
if [[ -z $user_id || $user_id == *$'\n'* ]]; then
  echo 'The new account was not found in the default local database; no role was granted.' >&2
  exit 1
fi
cargo run -p cloud-api -- grant-role "$user_id" admin local-dev-script
echo "Local test administrator created. Credentials saved in $credentials:"
cat "$credentials"
