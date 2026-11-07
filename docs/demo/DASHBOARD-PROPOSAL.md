# Dashboard: diseño implementado

La propuesta se integra ahora en `web/admin/src/views/Dashboard.vue`.

- Cabecera operativa con recarga y acceso a moderación.
- Conteos de publicadores y claves pendientes derivados de overview.
- Releases de la página cargada, indicando si existe otra página; no se presenta
  como total de la cola.
- Lista de próximas revisiones con namespace/plugin, versión, plataforma y estado.
- Actividad reciente con etiquetas humanas y códigos exactos.
- Distribución compacta de estados, sin gráficos de tendencias inventados.
- Errores y cargas independientes para overview, cola y auditoría; cola vacía
  diferenciada de una consulta fallida.
- Superficies planas, layout apilado móvil y temas claro/oscuro.

Los fixtures de demo son datos ficticios y mutables. `concept.html` es una referencia
histórica; no describe métricas o funcionalidades adicionales disponibles.
La validación integrada con backend real queda fuera de la demo local.
