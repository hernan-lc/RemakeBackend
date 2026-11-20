# Plan: login con códigos temporales y múltiples tipos de token

**Status:** fases A/B/C implementadas (reserve/relay/redeem + UI + keychain).
Fase D parcialmente cubierta con tests unitarios y E2E con mock; falta el E2E
con dos dispositivos reales. El canal futuro (SMTP/SMS/mobile) sigue sin
implementar, tal como se decidió.
**Fecha:** 2026-11-19.
**Research basis:** `better-auth-rs` pins `d4bb1ff` (backend) y `c694051`
(TikTools). El código OTP/magic-link/SMTP es prácticamente idéntico en ambos.

## 0. Decisiones del usuario (2026-11-19)

1. **SMTP se implementará en el futuro, no ahora.** El correo no se usa como
   canal de entrega en la fase actual de pruebas e implementación.
2. **Si el usuario ya tiene cuenta**, el código temporal es la vía más fácil.
   **Si no tiene cuenta**, se redirige / envía al flujo OAuth.
3. **Duración del código: 5 minutos** (la librería ya lo soporta). Está bien.
4. **El código se reenvía a cualquier dispositivo conectado** para el control
   remoto. Es decir, el canal de entrega actual son los dispositivos ya
   emparejados, no el correo.
5. **Si no hay otro dispositivo conectado (es el master), el control remoto no
   funcionará.** El flujo depende de que exista al menos un segundo dispositivo.
6. **La verificación entre dispositivos será configurable en el futuro** para
   objetivos como mobile, SMS o email. **Ignorar ahora**: son solo ejemplos de
   canales que podrían cambiar, no requisitos de esta fase.

## 1. Objetivo

Ofrecer login con un **código temporal de un solo uso** de 5 minutos, entregado
a los dispositivos conectados, sin guardar el token en claro y formalizando
varios tipos de token. Fallback a OAuth cuando no existe cuenta. No hace falta
"mucha seguridad" adicional: la librería ya aporta hash, TTL, un solo uso y
límite de intentos; nosotros solo cableamos el intercambio nativo.

## 2. Hallazgos clave

### 2.1 La librería ya lo implementa (no bloqueado por el pin)
- `verification.rs`: `verification:<purpose>:<sha256(token)>` en `_better_auth_kv`,
  `get_and_delete` atómico, TTL, `revoke_for_subject`.
- `passwordless.rs`: OTP de 6 dígitos, TTL 5 min, 5 intentos, hash con secreto,
  comparación en tiempo constante, namespaces por propósito. Magic links 5 min.
- `one_time_token.rs`: tokens de propósito, hasta 15 min, solo hash, revocables.
- `smtp.rs` y `email.rs`: transporte y plantillas (para el futuro; no se usan hoy).
- `router.rs`: rate limit por IP y cuenta, límite de intentos.

### 2.2 Estado actual de RemakeBackend
- `main.rs:301-315` instala solo `UsernamePlugin`; no hay plugins de código ni
  `EmailSender`. Las rutas de código no están expuestas hoy.
- No hay transporte de salida (SMTP/Resend/push) ni variables de correo.
- La librería traga errores de entrega (`router.rs:4190-4197`).
- `/sign-in/email-otp` crea sesión cookie, no bearer nativo: hace falta un
  endpoint nativo propio, como `desktop::redeem`.

### 2.3 "Nunca guardar el token" ya se cumple
- Backend: sesiones guardan `token_hash`; códigos como hash; JWT scoped no se
  guardan (firma + cutoff de revocación).
- TikTools: token en claro solo en keychain del SO y memoria; `AccountStatus`
  no-secreto; sidecar solo origin/cuenta/email/nombre.

## 3. Enfoque (fase actual)

**Código temporal computado por el host y reenviado a los dispositivos
conectados**, con vinculación a `reservationId` + `deviceNonce` + `APP_ORIGIN`.
El handoff PKCE existente se mantiene como camino device-bound ya probado.

### Tipos de token

| Tipo | Duración | Almacenamiento | Canal / vinculación |
|---|---|---|---|
| `handoff-code` (existe) | 60 s | solo hash SHA-256 | PKCE S256 + client id + loopback + generación |
| `login-code` (nuevo) | **5 min** | solo hash (secreto + reserva + nonce), `get_and_delete` | reenviado a dispositivos conectados; reserva + nonce + origin |
| `step-up-grant` (existe) | 5 min | UUID opaco en BD | usuario + sesión + acción + recurso |
| `delegated-credential` (existe) | 5 min | JWT Ed25519 no persistido | `origin`/`iss`/`aud`/scope + cutoff |

**Canales futuros (solo ejemplos, ignorar ahora):** mobile, SMS, email/SMTP,
magic-link. No son requisitos de esta fase y pueden cambiar.

### Flujo con cuenta existente

```text
Master (sin otros dispositivos): el control remoto NO funciona.
Con al menos un dispositivo conectado:

1. TikTools native -> cloud.login.code.reserve -> reservationId + deviceNonce
2. Host envía el código a los dispositivos conectados (canal de remote control)
3. Usuario lee/aprueba el código en otro dispositivo
4. cloud.login.code.submit(reservationId, email, code, deviceNonce)
5. Backend consume el hash, valida reserva/nonce/origen, crea sesión
6. Bearer -> finish_login() -> keychain del SO (nunca toca UI/IPC/SQLite)
```

- Reenviar código invalida el anterior.
- Cancelar sube la generación (mismo mecanismo de `desktop.rs`) e invalida reservas.
- Si **no existe cuenta**: redirigir al flujo OAuth en lugar de emitir código.

## 4. Fases

### Fase A — Backend (implementado)
1. ✅ `crates/cloud-api/src/login_code.rs` con `reserve`, `relay`, `redeem`:
   - `reserve` consulta el email en `user`; si no existe → `requires_oauth=true`,
     sin código. Genera `device_nonce`, guarda reservation con generación/origen.
   - `relay` emite un código de 6 dígitos, guarda solo
     `SHA256(secret‖reservation‖nonce‖code)`, y lo envía como frame
     `login.code` a las sesiones activas del usuario vía
     `Hub::route_to_session`. Si no hay dispositivos → `409`.
   - `redeem` consume el hash con `get_and_delete`, límite de 5 intentos,
     valida reserva/nonce/origen/generación, crea sesión Bearer con
     `SessionService::create`, devuelve `no-store`.
2. ✅ Rate limits (20/min reserve y redeem), body 1024 B, hash-only.
3. ✅ Sin plugins de email ni `EmailSender`.
4. ✅ Invalida códigos de handoff y reservations al canjear (generación).

### Fase B — TikTools (implementado)
4. ✅ `cloud-client/src/login_code.rs` con `reserve/relay/redeem` (sin token,
   `Zeroizing` para el código).
5. ✅ `tiktools-cloud-auth`: `PendingLoginCode`, mismas reservations y epoch,
   reutiliza `finish_login()` → solo keychain. `AccountStatus.loginCode` es
   no-secreto (email/expiry/entregas/requiresOauth).
6. ✅ `modules/cloud.rs`: `cloud.login.code.send/resend/submit` (usa
   `cloud.login.reserve` y `cloud.login.cancel` existentes).
7. ✅ UI: campo email, entrada de código, reenviar, cancelar y fallback OAuth
   automático cuando `requiresOauth`.
8. ✅ Keychain y `TokenStore` sin cambios.

### Fase C — Verificación
8. Tests: hash-only, single-use, 5 intentos, purposes separados, cross-origin,
   generación revocada, rate limits, y "master sin dispositivos = sin remoto".
9. E2E con dos dispositivos: reserve → relay → submit → keychain.

### Fase futura (no implementar ahora)
- SMTP/email o API HTTP, SMS, target mobile, verificación entre dispositivos
  configurable. Solo ejemplos de canales que pueden cambiar.

## 5. Notas de seguridad mínimas
- Mantener: un solo uso, TTL de 5 min, límite de intentos, rate limit y
  vinculación de dispositivo. No añadir TOTP ni segundo factor ahora.
- No guardar el bearer en claro: keychain del SO en cliente, hash en BD.
- Comprobar `origin` del código igual que en `sync_credentials.rs`.
- El código viaja solo por el canal ya autenticado de dispositivos conectados;
  el host nunca lo expone en control API, UI, logs ni SQLite.
