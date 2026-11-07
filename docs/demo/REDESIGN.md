# Propuesta de rediseño — TikTools Cloud

Estado: **propuesta**, no aplicada a los componentes Vue. La implementación de este
cambio se limita a la demo, sus comprobaciones y la documentación visual. El
[concepto](concept.html) presenta la dirección del dashboard; no es una aplicación
alternativa ni una promesa de datos nuevos disponibles en el backend.

## Dirección

Un espacio de operaciones centrado en la siguiente decisión: qué revisar, por qué
requiere atención y qué información falta. Conservar la base actual (navegación,
contraste de estados, modo oscuro, español/inglés), pero dar más jerarquía a las
acciones y reducir formularios permanentes. No introducir métricas de tendencias,
SLA o gráficos históricos sin datos reales del servidor.

## Observaciones de las capturas

| Evidencia | Observación | Cambio propuesto |
| --- | --- | --- |
| [Dashboard](screenshots/dashboard.png) | Cuatro cifras tienen peso similar; la tarjeta de estados queda casi vacía con pocos estados | Destacar trabajo pendiente y un único CTA «Revisar cola»; usar distribución compacta con enlaces a filtros |
| [Moderación](screenshots/moderation.png) | Cada release repite inspección, aprobación, rechazo y acciones adicionales; los hashes están colapsados | Lista compacta + detalle seleccionado con evidencia, estado de verificación y decisión contextual; confirmar una sola release |
| [Publicadores](screenshots/publishers.png) | Filtro, solicitud y revisión de claves conviven en la tabla | Bandeja por estado; detalle lateral para namespace, miembros y claves pendientes; explicar acciones no disponibles |
| [Auditoría](screenshots/audit.png) | Acciones y actores técnicos dominan la lectura | Etiqueta humana traducida junto al código original; actor, objeto, fecha y detalles expandibles; mantener IDs completos copiables |
| [Portal móvil](screenshots/portal-mobile.png) | Solicitud de namespace arriba, publicación extensa abajo y seis accesos en la barra inferior | Selector de namespace + resumen; «Nueva release» abre un flujo por pasos; navegación móvil con cuatro destinos y «Más» |
| [Claves](screenshots/portal-signing-keys.png) | Huellas largas compiten con acciones de gestión | Bloque de huella copiable y legible, estado y procedencia; nunca ocultar el valor completo durante aprobación |
| [Cuenta](screenshots/account.png) | Proveedores y cambio de username se muestran con peso similar | Secciones «Perfil» y «Acceso y seguridad», estado de proveedores y confirmación localizada |
| [Consentimiento](screenshots/desktop-consent.png) | Patrón visual distinto al admin y poco contexto de la aplicación destino | Compartir color, tipografía y controles; explicar «Conectar esta cuenta con TikTools Desktop» y mantener Cancelar visible |
| [Confirmación](screenshots/publisher-confirmation.png) | Confirmación genérica separada del contexto de la fila | Incluir objeto, alcance y consecuencia; foco inicial seguro y devolución del foco al activador |

La captura automatizada también detectó que `web/desktop/src/components/TextField.vue` muestra un `<label>` sin asociación al input. Priorizar `id`/`for` o envolver el control: el selector por nombre accesible no lo encuentra.

En el diálogo step-up, revisar también el texto «Your password never leaves this form»: la contraseña se envía al endpoint de verificación; el mensaje propuesto debe explicar que no se almacena en el navegador, sin prometer que no se transmite.

La franja amarilla se añade solo en demo y no se evalúa como diseño de producción.
Las capturas de demo no prueban permisos reales ni la disponibilidad de proveedores.

## Arquitectura de pantallas

1. **Inicio**: resumen de tareas pendientes, acceso a cola y actividad reciente.
   Las cifras deben enlazar al filtro correspondiente. Mostrar fecha de actualización
   y conservar la recarga manual. No convertir un total histórico en indicador urgente.
2. **Revisión**: separar pestañas «Publicadores», «Claves», «Releases». Lista a la
   izquierda y detalle a la derecha en escritorio; navegación lista → detalle en
   móvil. La decisión requiere evidencia y conserva revisión independiente y step-up.
3. **Mis publicadores**: selector de namespace, estado, rol y pestañas Resumen,
   Releases, Miembros, Claves, Configuración. Mover transferencia y eliminación a
   Configuración, sin hacer destructiva una acción primaria habitual.
4. **Publicación**: tres pasos, Paquete → Metadatos → Revisar y enviar. Mostrar
   validación, clave elegida, hash calculado y progreso; conservar límites y firma
   local. Los fallos deben conservar el contexto permitido y ofrecer reintento seguro.
5. **Auditoría**: cronología agrupada por día y tabla para comparación. La búsqueda
   actual es solo de la página cargada: mantener ese alcance explícito. Filtros
   globales y exportación se añadirían solo con soporte de API.
6. **Cuenta y autenticación**: misma cabecera, ancho de formulario y jerarquía en
   login, callback, 403, cuenta y consentimiento. No mostrar un proveedor no disponible.

## Sistema de componentes

| Componentes actuales | Evolución propuesta | Criterio de aceptación |
| --- | --- | --- |
| Shell, PageHeader | Navegación agrupada, breadcrumbs en detalle, una acción principal | Navegación por teclado; destino activo anunciado; no truncar etiquetas esenciales |
| AppButton, iconos | Primario, secundario, discreto, destructivo; altura táctil mínima 44 px | Nombre accesible en icon-only; carga anunciada; sin cambio de ancho al cargar |
| AppCard, StatCard, AppSectionHeader | Tarjetas por función y métricas enlazables; menos paneles anidados | Encabezados semánticos y jerarquía consistente |
| AppBadge, AppNotice, AppStatusBar | Estados semánticos compartidos con texto e icono | No depender del color; mismos términos EN/ES en cada pantalla |
| AppInput, AppSelect, AppSearch | Labels persistentes, ayuda y error asociados; validación inline | Error con `aria-describedby`; teclado y 200% zoom sin pérdida de controles |
| AppToolbar, AppActionTabs | Filtros activos visibles, recuento y limpiar; tabs solo cuando proceda | Teclado coherente con la semántica de grupo/tab elegida |
| StateBlock, EmptyState, AppSkeleton | Estados distintos: primera carga, actualización, vacío, sin coincidencias, error, acceso denegado | Recarga conserva datos válidos; error explica siguiente paso; anuncios no repetitivos |
| ConfirmDialog, StepUpDialog, TemporaryApprovalDialog | Cabecera contextual, consecuencias, CTA específico | Escape/cancelar, focus trap y retorno de foco; conservar reautenticación y advertencias de override |
| MemberManager, KeyManager, TransferPanel | Secciones con rol y permisos explícitos; acciones peligrosas agrupadas | No inferir autorización desde UI; servidor sigue siendo autoridad |
| ReleaseList, ReleaseWizard | Timeline de verificación y asistente por pasos | Sin ocultar firma, hash, rechazo o estado de cuarentena |
| ThemeToggle, LangSelect y campos desktop | Tokens y reglas compartidas sin acoplar runtime de ambas apps | Claro/oscuro, EN/ES y movimiento reducido coherentes |

Tokens propuestos: espaciado 4/8/12/16/24/32; radios 8 px en controles y 12 px en
paneles; fuente de sistema, cuerpo 14–16 px, títulos 24–32 px; índigo para acción,
pizarra para texto, verde/ámbar/rojo semánticos. Definir variables de superficie,
borde, texto secundario, foco y estados para ambos temas. Verificar contraste AA,
no asumirlo por la paleta. Hashes e IDs usan monoespaciada y salto de línea seguro.

## Entrega por fases

- **P1 — Base y navegación:** tokens, header, botones, formularios, estados y móvil.
  Validar 390, 768 y 1440 px, zoom 200%, teclado, temas y ambos idiomas.
- **P2 — Revisión:** lista/detalle y contexto de confirmación. Ejecutar tests de
  autorización, autoaprobación, sesión expirada y step-up con backend real.
- **P3 — Publicador:** selector y flujo de publicación; pruebas de interrupción,
  reintento, archivo inválido y cambios de rol. Separar configuración peligrosa.
- **P4 — Auditoría y acceso:** texto humano con códigos exactos, login, cuenta,
  consentimiento y errores. Validar callback y OAuth reales en entorno de prueba.

No hay cambios visuales publicados. La aprobación del concepto puede preceder a la
implementación por fases; la demo ya permite revisar las pantallas actuales.
