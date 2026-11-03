# TikTools Cloud

Early foundation for the optional TikTools cloud backend. See [specification](docs/SPECS.md), [plan](docs/PLAN.md), and [roadmap](docs/ROADMAP.md). This is not production-ready; publisher signing-key approval, registry publication, complete sync, desktop integration, and release gates remain outstanding. The per-surface status and limits are listed in the [feature matrix](docs/FEATURE-MATRIX.md).

The workspace declares Rust 1.95 or newer (required by the pinned TikTools package crates) and requires PostgreSQL. The minimums are verified with `cargo +1.95.0 check --workspace --locked` and `cargo +1.89.0 check -p cloud-client --all-features --locked`; CI repeats both checks. Better Auth is pinned to `c6940517155ff9a123221789b1efc89ef48cb19c`, with only `axum` and `sqlx` features; provider token retention is not enabled. Google credentials are optional and must be set together. Google JWKS is loaded at startup using the library's refresh policy. Production setup and independent security review remain necessary.

For local development, start `docker compose up -d`, create a private `.env` based on `.env.example`, export its variables in your shell, and run `cargo run -p cloud-api -- migrate` followed by `cargo run -p cloud-api`. Migrations are explicit and never run on server startup. The command applies Better Auth migrations followed by versioned SQLx Cloud migrations. `GET /health/live` checks the process, `/health/ready` checks database access, Cloud migration versions and checksums, and required auth and Cloud columns, and `/api/v1/me` requires a Better Auth session cookie. Authenticated users can request a pending namespace with `POST /api/v1/publishers` and list their own requests with `GET /api/v1/publishers`; a pending request grants no publishing or signing authority. Auth routes are mounted at `/api/auth/*`. The native handoff backend is experimental and is not yet connected to Desktop.

Back up the database before migrations. The upstream migration API does not supply an automatic rollback here; restore the backup to roll back. Do not use example credentials outside local development. Never log database URLs, auth secrets, cookies, or OAuth credentials.

## Tests

Run `cargo test --workspace --locked` for tests that do not need external services. For the OAuth and session integration test, create a dedicated disposable PostgreSQL database and run `CLOUD_TEST_DATABASE_URL=postgres://... cargo test --workspace --locked -- --ignored --test-threads=1`. This test applies Better Auth migrations and inserts fixture records; never point it at a production database. CI provisions a separate PostgreSQL service for it. The fixture is not a substitute for a real Google or desktop end-to-end test.

Publisher owners can manage existing users through `PUT /api/v1/publishers/{id}/members/{user_id}` with a JSON `role` of `admin`, `publisher`, or `viewer`, and remove them with `DELETE` on the same path. Members can list the publisher's members with `GET /api/v1/publishers/{id}/members`. Only the owner can change membership, and ownership cannot yet be transferred or removed. These roles do not bypass pending status or grant signing keys.

Registry test fixture and the proposed key lifecycle are documented in [registry operations](docs/REGISTRY-OPERATIONS.md). The fixture uses disposable keys and is not an official registry.

Google JWKS refresh defaults to one hour, with keys accepted for at most 24 hours and at least one minute between refresh attempts. `GOOGLE_JWKS_REFRESH_SECONDS`, `GOOGLE_JWKS_MAX_STALE_SECONDS`, and `GOOGLE_JWKS_MIN_RETRY_SECONDS` can adjust these bounds; invalid settings stop startup. Better Auth performs the refresh and rejects tokens once its key snapshot is too old. When configured values differ from its defaults, Cloud reloads Google's JWKS after applying the policy because the library replaces its cache; a failed reload stops startup. Live Google rotation and deployment behavior still require testing.

The experimental sync backend and opt-in client library currently support only plugin selections; see [sync API and limitations](docs/SYNC.md). It is not connected to Desktop or ready for sensitive data.

The experimental backend-mediated native sign-in flow and its limitations are described in [desktop authentication](docs/DESKTOP-AUTH.md).

Readiness returns 503 when the database is unavailable, a migration is missing, failed, changed, or unknown to the running binary, or required columns/CRUD permissions are absent. It does not prove that indexes, constraints, data, external providers, or all operations are healthy. Since newer migration versions make an older binary unready, deployment must account for that when rolling versions; migration upgrade and rollback rehearsals remain required.

Publisher approval and out-of-band Cloud role management are described in [publisher approval](docs/PUBLISHER-APPROVAL.md). Namespace approval does not delegate a signing key.

Archive verification and its boundaries are described in [package verification](docs/PACKAGE-VERIFICATION.md) and [release verification](docs/RELEASE-VERIFICATION.md). Publication is not yet implemented.

The experimental S3 quarantine upload API is documented in [object storage](docs/OBJECT-STORAGE.md). Uploaded objects can be submitted for integrity verification but are not yet published.

Publisher key enrollment and independent approval are described in [publisher keys](docs/PUBLISHER-KEYS.md).

Independent review decisions, release yanks, and security advisories are described in [moderation](docs/MODERATION.md); publication remains unimplemented.

The standalone [snapshot candidate builder](docs/REGISTRY-OPERATIONS.md#private-candidate-builder) uses the pinned registry protocol. An initial-candidate worker stages private file versions, and an operator command records audited promotion-readiness decisions; deployment remains unimplemented.
