# TikTools Cloud admin dashboard

Vite + Vue 3 + TypeScript app for Cloud admins and moderators. Same-origin
cookie sessions only; the backend (`cloud-api`) authorizes every call.

## Run

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
