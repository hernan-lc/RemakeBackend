# Scripts

Every script here is TypeScript. Run them with Node 22.18 or newer (native type stripping)
or with Bun; no build step is needed.

| Script | Purpose | Notes |
| --- | --- | --- |
| `create-local-admin.ts` | Creates a disposable local administrator in the default local database and grants the `admin` role | Needs `remake-dev dev --ui` running and `psql` on PATH. Writes credentials under `.local-dev/`. |
| `build-registry-fixture.ts` | Builds a disposable signed test registry from a pinned TikTools checkout | Test keys only. Refuses existing output and checkouts with tracked changes. |
| `verify-registry-fixture.ts` | Verifies a fixture with the pinned TikTools registry client | Runs offline against the checkout's `Cargo.lock`. |

CI runs the fixture pair in `.github/workflows/ci.yml` on Node 22.23.4.
