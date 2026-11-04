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
