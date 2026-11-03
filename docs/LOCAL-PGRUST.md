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
