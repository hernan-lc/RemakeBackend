# Dashboard: propuesta visual pendiente

El dashboard Vue actual no cambia en este PR. El acceso de escritorio sí está
implementado; esta propuesta se limita al panel de administración.

## Referencia actual

Escritorio de 1440 × 1000: rail lateral de 224 px; cuatro tarjetas para publicadores
pendientes, releases cargadas en la cola, claves pendientes y total de auditoría;
debajo, distribución de estados y eventos recientes. En móvil de 390 px, las métricas
se reparten en dos columnas y la navegación inferior tiene seis destinos.

La captura de esta revisión muestra 3 publicadores pendientes, 2 releases en la
página cargada, 2 claves pendientes y 4 eventos. Son fixtures mutables de la demo,
no métricas de producción. La tarjeta de distribución queda alta y casi vacía con
un solo estado; los códigos de auditoría ocupan la posición visual más destacada.

## Composición propuesta

- **Cabecera:** “Resumen de operaciones”, descripción breve, última actualización
  derivada de la consulta y Refresh discreto. CTA primario “Abrir cola de revisión”.
- **Primera fila:** tres tarjetas operativas (Publicadores pendientes, Claves
  pendientes, Releases cargadas). Mantener la precisión del contrato: el endpoint
  de moderación devuelve una página con `nextCursor`, no un total; no rotular esa
  cifra como total ni inventar tendencias. Si hay otra página, mostrar “N cargadas”.
- **Área principal, 2/3 del ancho:** tabla compacta “Próximas revisiones”, con
  namespace/plugin, versión, plataforma y estado, usando los registros de la cola.
  “Ver detalle” lleva al flujo de moderación. No aprobar desde una tarjeta resumen
  sin inspección ni ocultar la restricción de revisión independiente.
- **Columna secundaria, 1/3:** “Actividad reciente”, etiquetas humanas traducidas,
  actor y fecha; conservar el código exacto accesible en el detalle. El total de
  eventos pasa aquí y deja de competir con tareas pendientes.
- **Estados de releases:** distribución compacta al pie, barras proporcionales y
  números explícitos, sin altura mínima que produzca grandes vacíos. Cero y error
  deben ser diferentes; no convertir una consulta fallida en “sin trabajo”.
- **Navegación:** mantener el índigo, superficies pizarra, bordes sutiles y radios
  12 px. En móvil, usar Inicio, Revisión, Publicadores y Más; el contenido se apila
  manteniendo visible la acción primaria y controles de 44 px como mínimo.
- **Accesibilidad:** contraste AA verificado en ambos temas, foco visible, labels
  completos en EN/ES, zoom 200%, estados no dependientes solo del color y mensajes
  de recarga que no interrumpan al lector de pantalla.

El HTML `concept.html` ilustra una dirección previa, no funcionalidad integrada.
La propuesta final debe respetar el alcance de los datos anterior. Se puede usar
la captura actual como referencia para generar el mockup, conservando textos y
roles reales de administración; no reutilizar la pantalla de acceso de escritorio.
