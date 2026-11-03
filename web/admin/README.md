# TikTools Cloud admin dashboard

Vite + Vue 3 + TypeScript app for Cloud admins and moderators. Same-origin
cookie sessions only; the backend (`cloud-api`) authorizes every call.

## Run

For a coordinated local start, install the frontend dependencies with
`cd web/admin && npm ci`, then run from the repository root:

```sh
./scripts/dev-admin.sh
```

By default the launcher starts the Docker Compose database and applies
migrations. If Docker Compose is unavailable, it uses the Docker-free
[local pgrust launcher](../../docs/LOCAL-PGRUST.md), which has additional
build prerequisites. Database data persists between runs; it is not temporary.
A random local auth secret is saved in the
gitignored `.local-dev` directory so sessions survive restarts. Export
`DATABASE_URL` to use an existing database instead: in that case the launcher
does not start a database or run migrations. `AUTH_SECRET` and other backend
settings can also be exported. Back up existing data before migrations.

Open the printed URL, `http://127.0.0.1:5173/app`. The launcher sets the
backend origin and proxy target consistently and stops the API and UI when
either exits; the database remains running. Ports 3000 and 5173 must
be free; Vite fails instead of silently choosing another port.
Use the exact printed hostname: `localhost` and `127.0.0.1` are different
origins. Google OAuth also requires the corresponding callback URL to be
registered with Google.

To create a disposable administrator for the default local database, while
the launcher is running, execute `./scripts/create-local-admin.sh` from the
repository root. It creates a new random email and password each time, grants
the admin role, and saves the credentials in the gitignored `.local-dev`
directory. Use the printed credentials on the sign-in page.

To run each service separately:

```sh
cd web/admin
npm install
npm run dev      # http://127.0.0.1:5173, /api proxied to the backend
```

Open [http://127.0.0.1:5173/app](http://127.0.0.1:5173/app) in a browser.
`CLOUD_API_URL` (see `.env.example`) only changes the dev proxy target.
Production serves the built UI and the API from one HTTPS origin via
reverse proxy (see `deploy/admin-reverse-proxy.example.conf`), so the app
uses relative `/api/*` URLs and no backend address is baked in.

Backend for local work:

```sh
export DATABASE_URL=... AUTH_SECRET=... APP_ORIGIN=http://127.0.0.1:5173
cargo run -p cloud-api
```

`APP_ORIGIN` must match the browser origin or cookie/CSRF checks fail.

## What lives here

- `src/api/` — typed client (`/api/v1/me`, admin read APIs, moderation,
  publisher portal) plus pure auth helpers. Cookies via
  `credentials: 'include'`; no tokens in `localStorage`/`sessionStorage`.
- `src/views/` — `/login`, `/auth/callback`, `/app/*` (dashboard,
  publishers, moderation, audit, portal, account), `/403`, `/logout`.
- `src/i18n/` — Spanish/English strings (`en.ts`, `es.ts`).
- Pinia holds client-only UI state (locale, toasts). Server state (session,
  admin data) lives in TanStack Query and is invalidated on sign-out.

Signing-key enrollment is intentionally rendered **disabled**: key approval
needs session-bound step-up the backend does not offer yet, and this UI
must not bypass that gate.

## Checks

```sh
npm run typecheck   # vue-tsc
npm run build       # typecheck + vite build
npm test            # vitest: guards, auth outcomes, client, portal gate
npm run e2e         # playwright vs real backend + dev server (manual)
```
