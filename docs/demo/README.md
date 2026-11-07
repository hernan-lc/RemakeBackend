# Demo local y revisión visual

## Implementado

Modo demo explícito para las dos aplicaciones Vite. No requiere Rust, PostgreSQL,
OAuth, almacenamiento de archivos ni credenciales reales. El cliente HTTP y la autorización de producción no se modifican.
El acceso de escritorio incorpora ahora la jerarquía visual, las acciones Atrás/Entrar
y el estado cancelado descritos en `web/desktop/README.md`. El panel web incorpora el rediseño plano y responsive descrito en `REDESIGN.md`.

```sh
# Desde la raíz del repositorio
cd web/admin
npm ci
npm run demo
# http://127.0.0.1:5173/app
```

En otra terminal, para la pantalla de consentimiento de escritorio:

```sh
cd web/desktop
bun install --frozen-lockfile
npm run demo
# http://127.0.0.1:5174/desktop/login
```

Para mostrar el consentimiento, abrir `/desktop/login` con estos parámetros:
`redirectUri=http%3A%2F%2F127.0.0.1%3A9999%2Fcallback`,
`codeChallenge=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`,
`state=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb`.
El traspaso final al escritorio está bloqueado; no se genera ningún código real.

La demo empieza con una identidad ficticia autenticada. Después de cerrar sesión,
usar `demo@example.test` y `demo-only`. No ingresar contraseñas reales.
La sesión ficticia usa una cookie distinta de producción; los datos se mantienen
solo en memoria del proceso. Reiniciar Vite restablece todo. Los visitantes de una
misma instancia comparten los datos de ejemplo; usarla localmente, no como servicio.

| Disponible | Alcance |
| --- | --- |
| Dashboard, solicitudes, moderación, auditoría, portal, cuenta | Datos de ejemplo tipados; filtros y búsqueda existentes |
| Login y logout | Solo credenciales ficticias; sin tokens de producción |
| Solicitar/eliminar namespace nuevo | En memoria; el namespace con datos no puede eliminarse |
| Aprobar namespace ajeno; decidir una release ajena | En memoria; rechaza autoaprobación |
| Miembros, claves, transferencias, releases, formulario de subida | Lectura e inspección visual |
| OAuth, step-up, inscripción/revocación de claves, uploads, descarga de inspección, transferencias, cambios de miembros/cuenta, registro | Devuelven 501 `demo_operation_unavailable`; no simulan éxito |

El middleware solo se activa con `--mode demo`, intercepta **todas** las rutas
`/api/` y desactiva el proxy del panel. Una ruta desconocida no cae al backend.
`vite build --mode demo` falla intencionalmente. `npm run build` continúa generando
el cliente normal. La demo no prueba seguridad del backend ni reemplaza sus tests.
No se modifican ni despliegan servicios de producción.

## Evidencia y reproducción

Las capturas, la galería y los informes son artefactos locales ignorados por Git.
Los enlaces de evidencia en esta guía y la propuesta funcionan después de ejecutar
los scripts siguientes. El concepto HTML es fuente revisable y sí se versiona.

- [Galería de capturas](gallery.html): pantallas actuales, escritorio/móvil y oscuro.
- [Dashboard](DASHBOARD-PROPOSAL.md): composición implementada y alcance de datos.
- [Rediseño integral](REDESIGN.md): cambios implementados y límites de validación.
- [Concepto visual](concept.html): maqueta estática propuesta, no integrada en Vue.
- [Informe de navegador](screenshots/report.json): errores JS y desbordamiento horizontal.

Con ambos servidores demo activos, desde `web/admin`:

```sh
node demo/capture.mjs
node demo/capture-desktop.mjs
node demo/gallery.mjs
```

Los scripts usan Playwright y `/usr/bin/chromium`; puede seleccionarse otro binario
con `CHROMIUM_PATH`. Reiniciar la demo antes de repetir las capturas para recuperar
los datos iniciales. La captura del panel también comprueba solicitud de namespace,
logout y login desde la interfaz. Las capturas completas de páginas largas incluyen
la barra móvil fija en su posición del viewport; no representan toda la página
visible a la vez.

Validación realizada: build de producción del panel; 56 tests en 14 archivos;
build y typecheck de escritorio con actualización de assets embebidos;
rechazo de build demo; búsqueda de fixtures en el bundle normal sin coincidencias;
recorrido de navegador sobre las pantallas capturadas. GitHub respondió HTTP 200 y
el entorno confirmó red `unrestricted/enforced` y estado `running/connected`.

Limitaciones: capturas con contenido representativo, no todas las permutaciones de
permisos, errores y tamaños. Los flujos reales de OAuth, firma y almacenamiento
requieren backend para una validación integral. No hay bloqueo para probar la demo.
