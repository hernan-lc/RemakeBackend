# Rediseño web implementado

El panel Vue y el acceso de escritorio incorporan superficies planas, bordes
legibles, controles principales de 44 px y jerarquía de acciones. Se eliminaron
sombras decorativas; no se añadieron degradados. `concept.html` se conserva como
referencia histórica, no como una segunda aplicación.

## Pantallas y componentes

- **Navegación:** rail de escritorio agrupado en Operaciones y Publicación;
  móvil con Inicio, Revisión, Publicadores y Más. Más permite llegar a auditoría,
  revisión de publicadores y cuenta; Escape cierra el desplegable y devuelve foco.
- **Dashboard:** resumen operativo, registros reales de la cola, actividad con
  etiquetas legibles y códigos originales, distribución compacta. Carga, vacío y
  error son independientes por consulta. La cantidad de releases corresponde a
  la página cargada, no a un total inventado.
- **Moderación:** lista y detalle seleccionado, hashes y huellas completos,
  inspección y decisiones contextuales; conserva revisión independiente.
- **Publicadores:** acciones compactas, estados legibles y confirmaciones con
  contexto. Conserva comprobaciones de membresía y permisos.
- **Auditoría:** nombres de eventos conocidos traducidos, códigos originales y
  detalles completos expandibles. La búsqueda conserva su alcance existente.
- **Portal:** selector de namespace y paneles correspondientes, solicitud aparte,
  miembros, claves, transferencias y releases con superficies planas. El asistente
  desplegable tiene Paquete, Metadatos y Revisión; valida archivo y ámbito antes
  de avanzar, mueve el foco al paso activo y bloquea cambios durante envío.
- **Cuenta y acceso:** perfil separado de seguridad, login, callback, 403 y logout
  coherentes. Consentimiento de escritorio mantiene identidad única, Atrás y
  Cancelar diferenciados y cancelación terminal sin formulario activo.
- **Base compartida:** tarjetas, botones, formularios, búsqueda, pestañas, avisos,
  skeletons y diálogos sin sombras decorativas. Los campos disabled desactivan
  el control nativo. El texto step-up explica correctamente que la contraseña se
  envía para verificar y no se guarda en el almacenamiento del navegador.

Se conservan rutas, contratos API, permisos, confirmaciones y step-up. El servidor
sigue siendo autoridad. No se añadieron métricas históricas, exportación global,
proveedores ni capacidades que las APIs no ofrecen. EN/ES y ambos temas cubren los
nuevos textos y estados del panel.

## Validación y límites

Builds de producción de ambas aplicaciones y 56 tests del panel. Recorridos de
navegador para acceso de escritorio, pasos del asistente, errores independientes
 y cola vacía. Revisión visual y axe WCAG A/AA sobre nueve rutas a 390 y 1440 px,
claro y oscuro. Las capturas e informes son artefactos locales, no fuente.

Estas comprobaciones no certifican accesibilidad exhaustiva ni seguridad del
backend. OAuth, step-up y almacenamiento reales requieren un entorno integrado;
la demo rechaza esas operaciones. No se hizo merge ni despliegue.
