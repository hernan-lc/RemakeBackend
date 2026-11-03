# Local pgrust without Docker

The launcher builds the pinned pgrust source in `vendor/pgrust` locally. The
source is a Git submodule so it retains its own workspace and can be developed
and contributed upstream independently. Initialize it after cloning:

The build checks for the repository's mimalloc compatibility fix for an early
`free(NULL)` crash on glibc 2.44. If needed, it applies the patch tracked in
`patches/pgrust-mimalloc-glibc-2.44.patch` to the submodule working tree. The
pinned submodule commit already includes the fix. If the patch conflicts with
local submodule edits or a different revision, the build stops for review.

```bash
git submodule update --init --recursive
cp .env.example .env
# Edit .env and set a private AUTH_SECRET.
set -a; . ./.env; set +a
cargo run -p pgrust-local -- up
cargo run -p cloud-api -- migrate
cargo run -p cloud-api
```

You need Git, the Rust toolchain specified by the submodule, a C/C++ build
chain, pkg-config, RE2 development files, and `psql` on PATH. Cargo may download
source dependencies on the first build. No Docker or prebuilt pgrust download
is required. See the submodule README for platform-specific build prerequisites.
The launcher raises the server process's stack limit to 65520 KiB as required
by pgrust; the shell's hard stack limit must allow at least that much.

`cargo run -p pgrust-local -- install` builds the source; `up` builds it if
needed and starts the server; `status` reports its state; `down` stops it while
preserving data. `cargo test -p pgrust-local --locked` runs launcher unit tests.

The cluster and logs reside under `~/.local/share/remakebackend/pgrust`, or
`$XDG_DATA_HOME/remakebackend/pgrust`; `PGRUST_LOCAL_HOME` overrides the root.
The launcher only listens on loopback and refuses an occupied port. The existing
cluster is reused; changing credentials does not rotate them. Migrations are
explicit and no reset command is provided. Use a disposable, dedicated database
for integration tests. pgrust is beta software and is not production-ready.

The source is licensed under its upstream license; see `vendor/pgrust/LICENSE`.

## Alternative: prebuilt download via `scripts/pgrust-local.py`

If you cannot build from source, the stdlib-only Python script
`scripts/pgrust-local.py` downloads and verifies the official pgrust
v0.4-beta bundle instead. It needs Python 3.9+, Linux (glibc 2.34+) or
macOS 11+, and network access for the initial download. There is no native
Windows release for v0.4-beta; use WSL2 on Windows. Do not mix a cluster
created by one launcher with the other; both default to the same data root.

```bash
cp .env.example .env
# Edit .env: use a local database URL and change AUTH_SECRET.
set -a; . ./.env; set +a  # Export env vars for the backend and launcher
python3 scripts/pgrust-local.py up
cargo run -p cloud-api -- migrate
cargo run -p cloud-api
```

The runner reads `DATABASE_URL` from the environment first, then `.env`.
For the checked-in example it creates the `cloud` role as the cluster
superuser and a separate `cloud` database, matching:
`postgres://cloud:local-only-password@localhost:5432/cloud`.

```bash
python3 scripts/pgrust-local.py install  # Verify checksum and unpack official tarball
python3 scripts/pgrust-local.py up       # Install/init if needed; start and create database
python3 scripts/pgrust-local.py status   # Show managed process and data path
python3 scripts/pgrust-local.py down     # SIGINT/fast shutdown; preserve data
python3 -m unittest discover -s tests -p 'test_pgrust_local.py'
```

The password is supplied only through the temporary, mode-0600
`initdb --pwfile` and the `PGPASSWORD` environment variable for `psql`; it is
not included in the process command line or stored by this tool. Both TCP and
local socket connections use SCRAM authentication. TCP binds only 127.0.0.1; no
public listener is configured. The tool refuses to reuse an occupied TCP port
or clobber an existing nonempty data directory.

**Important:**

- Existing `data/PG_VERSION` is reused without modifying it. Changing the
  user/password in `.env` does not rotate database credentials. Do not
  mix this cluster with Docker's separate volume.
- Migrations remain **explicit**; `up` does not run application migrations.
  Run `cargo run -p cloud-api -- migrate` after starting the database.
- Do not run `docker compose up` simultaneously against the same local port.
- For failed startup inspect `pgrust.log` in the data root. `down` only
  signals the verified managed process; it is deliberately not a general
  PostgreSQL process killer. No automated deletion/reset command is provided.
- This runs a pinned beta release and does not automatically upgrade or
  migrate old clusters. Back up and restore before changing engine versions.
- pgrust's own README says it is not production-ready. This helper is for
  local development and disposable testing, not an endorsed production setup.

Backend configuration and recovery guidance: `README.md` and
`docs/DATABASE-RECOVERY.md`. Upstream:
https://github.com/malisper/pgrust#quickstart
