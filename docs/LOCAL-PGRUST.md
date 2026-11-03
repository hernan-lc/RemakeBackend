# pgrust without Docker (local development)

This is an **optional** local runner for the same pgrust v0.4-beta engine used by
\`compose.yaml\`. It is **not** a second database implementation, and it does
not install, require, or modify Docker. Official Linux and macOS bundles include
\`postgres\`, \`initdb\`, \`psql\`, and required share files. There is no native
Windows release for v0.4-beta; use WSL2 on Windows.

Requirements: Python 3.9+ (stdlib only), Linux (glibc 2.34+ for Linux binary)
or macOS 11+, and network access for the initial download.

From the repository root:

\`\`\`bash
cp .env.example .env
# Edit .env: use a local database URL and change AUTH_SECRET.
python3 scripts/pgrust-local.py up
cargo run -p cloud-api -- migrate
cargo run -p cloud-api
\`\`\`

The runner reads \`DATABASE_URL\` from the environment first, then \`.env\`.
For the checked-in example it creates the \`cloud\` role as the cluster
superuser and a separate \`cloud\` database, matching:
\`postgres://cloud:local-only-password@localhost:5432/cloud\`.

Commands:

\`\`\`bash
python3 scripts/pgrust-local.py install  # Verify checksum and unpack official tarball
python3 scripts/pgrust-local.py up       # Install/init if needed; start and create database
python3 scripts/pgrust-local.py status   # Show managed process and data path
python3 scripts/pgrust-local.py down     # SIGINT/fast shutdown; preserve data
python3 -m unittest discover -s tests -p 'test_pgrust_local.py'
\`\`\`

The install, data, log and PID file are stored under
\`~/.local/share/remakebackend/pgrust\` by default (or under
\`$XDG_DATA_HOME/remakebackend/pgrust\`). Set \`PGRUST_LOCAL_HOME\`
to override the root. The password is supplied only through the temporary,
mode-0600 \`initdb --pwfile\` and the \`PGPASSWORD\` environment variable for
\`psql\`; it is not included in the process command line or stored by this tool.
Both TCP and local socket connections use SCRAM authentication. TCP binds only
127.0.0.1; no public listener is configured. The tool refuses to reuse an
occupied TCP port or clobber an existing nonempty data directory.

**Important:**

- Existing \`data/PG_VERSION\` is reused without modifying it. Changing the
  user/password in \`.env\` does not rotate database credentials. Do not
  mix this cluster with Docker's separate volume.
- Migrations remain **explicit**; \`up\` does not run application migrations.
  Run \`cargo run -p cloud-api -- migrate\` after starting the database.
- Do not run \`docker compose up\` simultaneously against the same local port.
- For failed startup inspect \`pgrust.log\` in the data root. \`down\` only
  signals the verified managed process; it is deliberately not a general
  PostgreSQL process killer. No automated deletion/reset command is provided.
- This runs a pinned beta release and does not automatically upgrade or
  migrate old clusters. Back up and restore before changing engine versions.
- pgrust's own README says it is not production-ready. This helper is for
  local development and disposable testing, not an endorsed production setup.

Backend configuration and recovery guidance: \`README.md\` and
\`docs/DATABASE-RECOVERY.md\`. Upstream:
https://github.com/malisper/pgrust#quickstart
