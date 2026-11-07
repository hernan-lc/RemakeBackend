# Desktop consent page

TypeScript + Vite source for the browser page the desktop handoff serves at
`/desktop/login`. The API binary embeds the built bundle, so the checked-in
`crates/cloud-api/web/dist/` files are build output: edit `src/`, rebuild,
and commit both.

```bash
cd web/desktop
bun install
bun run typecheck
bun run build   # writes crates/cloud-api/web/dist/{index.html,login.js,login.css}
```

`bun run dev` serves the page standalone for iteration (API calls still go
to the same origin, so point it at a running backend or use the built page
through the API). Keep the bundle dependency-free and CSP-compatible:
no inline scripts, no external resources, system fonts only.

## Local demo

Run `npm run demo` after dependency installation. See the
[demo guide](../../docs/demo/README.md) for the consent URL, fictional credentials
and limitations. The final desktop handoff is intentionally unavailable.

## Access-flow browser checks

With the desktop Vite server running and `web/admin` dependencies installed:

```sh
node web/admin/demo/test-desktop-flow.mjs # from the repository root
```

The test intercepts all API requests with isolated fixtures. It checks login errors
and success, Back and keyboard focus, cancellation while requests are in flight,
invalid handoff/callback rejection, and registration layouts in all five locales.
Set `CAPTURE_DIR` to an absolute directory to save reference screenshots. These
checks do not replace integration tests against real OAuth and desktop callbacks.
