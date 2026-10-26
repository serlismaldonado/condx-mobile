//! La app en el idioma del teléfono: en español si el teléfono está en español; si no, en inglés (los
//! textos del código). Como la app de escritorio, que sigue el idioma del sistema.

/// Si el teléfono está en español.
pub fn spanish() -> bool {
    nsh_core::ui::ambient::locale().as_str().to_ascii_lowercase().starts_with("es")
}

/// El texto en el idioma del teléfono (el mismo, si no hay traducción).
pub fn t(en: &str) -> String {
    if spanish() {
        if let Some((_, es)) = ES.iter().find(|(k, _)| *k == en) {
            return es.to_string();
        }
    }
    en.to_string()
}

const ES: &[(&str, &str)] = &[
    // Pestañas y barra de arriba.
    ("Threads", "Hilos"),
    ("Approvals", "Aprobaciones"),
    ("Jobs", "Trabajos"),
    ("Envs", "Entornos"),
    ("Incidents", "Incidentes"),
    ("Settings", "Ajustes"),
    // Ajustes.
    ("Appearance", "Apariencia"),
    ("System", "Sistema"),
    ("Light", "Claro"),
    ("Dark", "Oscuro"),
    ("Agents answer in", "Los agentes contestan en"),
    ("Automatic", "Automático"),
    ("Automatic: each agent answers in the language of the message.", "Automático: cada agente contesta en el idioma del mensaje."),
    ("Every agent, on the Mac and in the cloud site, answers in this language.", "Todos los agentes, en la Mac y en el sitio en la nube, contestan en este idioma."),
    ("This phone", "Este teléfono"),
    ("Key", "Llave"),
    ("Keys", "Claves"),
    ("Stored in", "Guardada en"),
    ("No provider keys yet.", "Todavía no hay claves de proveedores."),
    ("Pair this phone again to receive provider keys.", "Vincula este teléfono de nuevo para recibir las claves de los proveedores."),
    ("Unpair this phone", "Desvincular este teléfono"),
    // Vincular.
    ("Pair with your Mac", "Vincular con tu Mac"),
    ("Confirm on your Mac", "Confirma en tu Mac"),
    ("Check that the Mac shows this same key, then press Confirm there.", "Comprueba que la Mac muestra esta misma llave y confirma allá."),
    // Hilos.
    ("No threads yet", "Todavía no hay hilos"),
    ("Flows with agents (Condx → Workflows) open one per run. You can follow them and answer here.", "Los flujos con agentes (Condx → Flujos) abren uno por corrida. Aquí los sigues y contestas."),
    ("Loading more…", "Cargando más…"),
    ("Start a conversation", "Empieza una conversación"),
    ("Ask the agents about your project: they read your code and what Condx knows (environments, jobs, incidents, flows). You can attach a photo or send a voice note.", "Pregúntales a los agentes por tu proyecto: leen tu código y lo que sabe Condx (entornos, trabajos, incidentes, flujos). Puedes adjuntar una foto o mandar una nota de voz."),
    ("The agents answer here", "Los agentes contestan aquí"),
    ("New thread", "Hilo nuevo"),
    ("Message the agents…", "Escríbeles a los agentes…"),
    ("Answer the question…", "Contesta la pregunta…"),
    ("Attachment", "Adjunto"),
    ("Queued · goes out when an agent is free", "En cola · sale cuando un agente se libere"),
    ("Working…", "Trabajando…"),
    ("Thinking…", "Pensando…"),
    ("Reading the code…", "Leyendo el código…"),
    ("Running a command…", "Corriendo un comando…"),
    ("Editing files…", "Editando archivos…"),
    ("Checking the project…", "Revisando el proyecto…"),
    ("Writing…", "Escribiendo…"),
    ("now", "ahora"),
    ("you", "tú"),
    ("Replied", "Respondió"),
    ("Working", "Trabajando"),
    ("Queued", "En cola"),
    ("Needs your answer", "Espera tu respuesta"),
    ("Needs your approval", "Espera tu aprobación"),
    ("Done", "Listo"),
    ("Failed", "Falló"),
    ("Stopped", "Detenido"),
    ("Cancelled", "Cancelado"),
    ("New", "Nuevo"),
    // Aprobaciones.
    ("Nothing is waiting for your approval.", "Nada espera tu aprobación."),
    ("Slide to approve", "Desliza para aprobar"),
    ("Reject", "Rechazar"),
    ("Approved from this phone", "Aprobado desde este teléfono"),
    ("Rejected from this phone", "Rechazado desde este teléfono"),
    ("Waiting for Touch ID on the Mac", "Espera Touch ID en la Mac"),
    ("Sending your decision…", "Enviando tu decisión…"),
    ("Production: Touch ID on the Mac", "Producción: Touch ID en la Mac"),
    ("Decided, or no longer waiting.", "Decidido, o ya no espera."),
    ("Waiting for the Mac…", "Esperando a la Mac…"),
    // Incidentes.
    ("No incidents. Everything Condx watches is fine.", "Sin incidentes. Todo lo que vigila Condx está bien."),
    ("Sending your request…", "Enviando tu pedido…"),
    ("Open the PR", "Abrir el PR"),
    ("Fix", "Arreglo"),
    ("Turn on a workflow on the Mac to deploy fixes from here.", "Enciende un flujo en la Mac para desplegar arreglos desde aquí."),
    ("No PR", "Sin PR"),
    ("With a PR", "Con PR"),
    ("Opens a PR with the fix first.", "Primero abre un PR con el arreglo."),
    ("Deploys directly, without a PR.", "Despliega directo, sin PR."),
    ("Nothing else to do here.", "Nada más que hacer aquí."),
    // Trabajos.
    ("No jobs yet", "Todavía no hay trabajos"),
    ("Deploys, functions, backups and releases the Mac runs show up here.", "Aquí aparecen los despliegues, funciones, respaldos y versiones que corre la Mac."),
    ("Run a job", "Correr un trabajo"),
    ("Run a flow", "Correr un flujo"),
    ("Environment", "Entorno"),
    ("What to run", "Qué correr"),
    ("Flow", "Flujo"),
    ("Nothing can run on this environment from here.", "Desde aquí no se puede correr nada en este entorno."),
    ("No flows turned on. Create or turn one on in Condx on the Mac (Workflows).", "No hay flujos encendidos. Crea o enciende uno en Condx en la Mac (Flujos)."),
    ("Production: the Mac asks for Touch ID before it runs.", "Producción: la Mac pide Touch ID antes de correrlo."),
    ("Sending your request to the Mac…", "Enviando tu pedido a la Mac…"),
    ("Cancel", "Cancelar"),
    ("Running", "Corriendo"),
    ("Needs approval", "Espera aprobación"),
    ("Expired", "Venció"),
    ("Rejected", "Rechazado"),
    // Entornos.
    ("No environments yet", "Todavía no hay entornos"),
    ("The Mac publishes its environments here once it's linked to Condx Cloud.", "La Mac publica aquí sus entornos cuando está vinculada a Condx Cloud."),
    ("Responding", "Responde"),
    ("Not responding", "No responde"),
    ("Not checked yet", "Sin revisar todavía"),
    ("production", "producción"),
    ("cloud dev", "nube dev"),
    ("Waiting for your answer", "Espera tu respuesta"),
    ("Waiting for your approval", "Espera tu aprobación"),
    ("Waiting for you", "Te espera"),
    ("Preparing a fix…", "Preparando un arreglo…"),
    ("Diagnosing…", "Diagnosticando…"),
    ("Fix ready", "Arreglo listo"),
    ("Fix attempt done", "Intento de arreglo terminado"),
    ("Diagnosis ready", "Diagnóstico listo"),
    ("Agent stopped (limit or budget)", "Agente detenido (límite o presupuesto)"),
    ("Agent stopped", "Agente detenido"),
    ("Agent failed", "El agente falló"),
    ("Thread", "Hilo"),
    ("Code (K7Q2-M9XD-4HWR) or the QR's text", "Código (K7Q2-M9XD-4HWR) o el texto del QR"),
    ("Deployment URL (only with a typed code)", "URL del despliegue (solo con código escrito)"),
    ("Agent", "Agente"),
    // Resumen y servicios.
    ("Online", "En línea"),
    ("To approve", "Aprobar"),
    ("Attention", "Atención"),
    ("Local services", "Servicios locales"),
    ("Start all", "Levantar todo"),
    ("Stop all", "Detener todo"),
    ("Start", "Levantar"),
    ("Stop", "Detener"),
    ("Restart", "Reiniciar"),
    ("Starting…", "Levantando…"),
    ("Stopping…", "Deteniendo…"),
    ("Restarting…", "Reiniciando…"),
    ("Port in use", "Puerto ocupado"),
    ("Clean", "Sin cambios"),
    ("idle", "en espera"),
    ("deploying", "desplegando"),
    ("syncing", "sincronizando"),
    ("starting", "arrancando"),
    ("error", "error"),
    ("The Mac is off: its local services can be started when it's back.", "La Mac está apagada: sus servicios locales se manejan cuando vuelva."),
    // Avisos.
    ("Install", "Instalar"),
];

/// Un texto con datos en los dos idiomas: `tr(format!(…), format!(…))`.
pub fn tr(en: String, es: String) -> String {
    if spanish() { es } else { en }
}
