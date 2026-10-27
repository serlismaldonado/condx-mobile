//! Condx en el teléfono: vincularlo con tu Mac y aprobar (o rechazar) lo que espera tu aprobación.
//!
//! - **Vincular**: el código que muestra la Mac (con su QR). El teléfono genera su llave Ed25519 y su
//!   token, reclama el código y espera a que confirmes en la Mac (comparando la huella).
//! - **Identidad**: Keychain en iOS, Keystore en Android (`secure.rs`).
//! - **Aprobar**: la decisión sale firmada; la Mac la verifica con la llave que confirmó. En
//!   producción el teléfono solo propone: la Mac confirma con Touch ID.
//!
//! La red y el almacenamiento corren fuera del hilo de la interfaz (`blocking`).

pub mod audio;
pub mod bell;
pub mod chat;
pub mod i18n;
pub mod live;
pub mod logos;
pub mod md;
pub mod ops;
pub mod scan;
pub mod secure;
pub mod sync;
pub mod theme;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_base::safe_area_view;
use nsh_core::ui::prelude::*;
use sync::{Approval, Incident, Phone, ProviderKey, Thread};
use theme::{c, card, field, kind_tag, kv, mono, muted, section, status_tag, tokens, Health, Variant};

#[native_module("condx-mobile/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    // IBM Plex Sans y JetBrains Mono, las fuentes de la app de escritorio.
    registry.assets(assets!());
}

#[derive(Clone, PartialEq)]
enum Screen {
    Loading,
    Pair,
    /// Esperando la confirmación en la Mac: la huella para comparar.
    Waiting(String),
    Ready,
}

type Slot = Arc<Mutex<Option<Phone>>>;

const DEVICE_NAME: &str = if cfg!(target_os = "ios") { "iPhone" } else if cfg!(target_os = "android") { "Android phone" } else { "Desktop test" };

/// El último pánico dentro de un acceso al teléfono (se ve en Ajustes): antes envenenaba el candado y
/// desde ahí nada salía (ni los mensajes) sin ningún error a la vista.
pub static LAST_PANIC: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

async fn with_phone<R: Send + 'static>(slot: &Slot, f: impl FnOnce(&mut Phone) -> R + Send + 'static) -> Option<R> {
    let slot = slot.clone();
    blocking(move || {
        // Un pánico anterior no deja el teléfono inservible: se sigue con el estado que quedó.
        let mut guard = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let phone = guard.as_mut()?;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(phone))) {
            Ok(r) => Some(r),
            Err(e) => {
                let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into());
                if let Ok(mut last) = LAST_PANIC.lock() {
                    *last = msg.chars().take(200).collect();
                }
                None
            }
        }
    })
    .await
    .ok()
    .flatten()
}

/// Sin latido de la Mac en este rato, se la da por apagada (late cada minuto; al irse en orden avisa
/// con 0 y se ve enseguida).
const MAC_OFFLINE_MS: i64 = 3 * 60_000;

#[component]
pub fn App() -> impl IntoElement {
    // El tema elegido en Ajustes (sistema, claro u oscuro).
    let scheme = signal(theme::saved_scheme());
    theme::apply_scheme(&scheme.get(), false);
    // En qué idioma contestan los agentes (lo configura la Mac).
    let agent_lang = signal(String::new());
    // Agentes de fuera conectados al MCP remoto (Claude en cualquier lugar): la lista, la URL del conector
    // y el formulario para conectar otro (nombre, nivel y el código que sale).
    let mcp_conns = signal(Vec::<(String, String, String, i64)>::new());
    let mcp_url = signal(String::new());
    let mcp_loaded = signal(false);
    let mcp_form = signal(false);
    let mcp_name = signal("Claude".to_string());
    let mcp_level = signal("orchestrate");
    let mcp_code = signal(None::<Result<(String, String), String>>);
    let screen = signal(Screen::Loading);
    // El dueño de la app: las tareas que arranca un botón corren bajo él (las del botón se cancelan
    // cuando ese pedazo de la interfaz se vuelve a dibujar, p. ej. al empezar a grabar).
    let app_owner = Owner::current();
    // Mensajes para la persona (errores, avisos) y el estado de la sincronización (barra superior).
    let status = signal(String::new());
    let sync_line = signal(String::new());
    // Lo que no salió: cambios esperando y los que la nube rechazó (Ajustes → Este teléfono).
    let outbox = signal((0usize, Vec::<String>::new(), String::new()));
    // Qué pasó con el último mensaje que se mandó desde un hilo (para ver por qué no sale).
    let last_send = signal(String::new());
    let presence = signal(chat::Presence::Mac);
    let waiting_count = signal(0usize);
    // Avanza en cada vuelta del ciclo: los «hace N min» se vuelven a calcular.
    let tick = signal(0u64);
    let approvals = signal(Vec::<Approval>::new());
    // Incidentes (A0b): la lista, la pestaña elegida, las tarjetas abiertas y el mensaje al agente.
    let incidents = signal(Vec::<Incident>::new());
    let tab = signal("threads");
    let expanded = signal(Vec::<String>::new());
    // El menú «⋯» abierto (uno a la vez), las respuestas que se ven enteras y el incidente cuyo hilo
    // se pidió (se abre cuando el daemon contesta «thread <id>»).
    let menu_open = signal(Option::<String>::None);
    let answer_open = signal(Vec::<String>::new());
    let pending_thread = signal(Option::<String>::None);
    // Sincronizar ya (se mandó algo).
    let kick = signal(false);
    // El flujo elegido en «⋯» para desplegar un arreglo: (incidente, flujo, nombre); se confirma deslizando.
    let deploy_pick = signal(Option::<(String, String, String)>::None);
    // Desplegar sin PR (`<flujo>|direct`): por defecto el daemon abre el PR antes.
    let deploy_direct = signal(false);
    // Hilos (F6): las corridas de los flujos con sus agentes; el que está abierto y lo que se escribe.
    let threads = signal(Vec::<Thread>::new());
    // Los entornos y los trabajos recientes de la Mac (solo lectura) y el trabajo abierto.
    let envs = signal(Vec::<sync::Env>::new());
    let jobs = signal(Vec::<sync::Job>::new());
    let job_open = signal(None::<String>);
    // El resumen de cada proyecto (servicios, git, lo que pide atención) y si se pidió algo a un servicio.
    let overview = signal(serde_json::Value::Null);
    let service_asked = signal(false);
    let site_overview = signal(serde_json::Value::Null);
    let open_projects = signal(theme::pref("openProjects").as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default());
    // Pedir correr una receta o un flujo: el formulario, el catálogo de la Mac y cómo va lo pedido.
    let run_form = ops::RunForm::new();
    let run_catalog = signal((Vec::<sync::RecipeInfo>::new(), Vec::<sync::FlowInfo>::new()));
    let run_state = signal(sync::RunState::default());
    // La lista de la pantalla principal: al cambiar de pestaña vuelve arriba.
    let main_scroll = element_ref();
    // Ya llegó algo (de lo guardado o de la nube): sin esto, una lista vacía es «cargando» (skeleton).
    let loaded = signal(false);
    // Cuántos hilos se muestran: 20 y 20 más al acercarse al final.
    let shown = signal(20usize);
    let thread_open = signal(Option::<String>::None);
    // La fila «_new» (pedir un hilo nuevo) tal como la ve este teléfono.
    let new_thread = signal(Option::<Thread>::None);
    let thread_text = signal(String::new());
    // Adjuntos del próximo mensaje, la nota de voz que se graba y el menú de adjuntar.
    let thread_files = signal(Vec::<chat::PendingFile>::new());
    let recording = signal(Option::<f64>::None);
    let attach_menu = signal(false);
    let catalog = signal(Vec::<(String, String, String)>::new());
    let agent_models = signal(Vec::<(String, Option<String>, Vec<String>)>::new());
    let projects = signal(Vec::<String>::new());
    let default_models = signal((String::new(), Vec::<String>::new()));
    // Una versión nueva de esta app (la publica el daemon): versión, número y enlace.
    let apk = signal(None::<(String, i64, String)>);
    // El mensaje que se está respondiendo (deslizando), hasta mandar o cancelar.
    let thread_reply = signal(chat::ReplyTo::None);
    // Claves de proveedores cifradas para este teléfono (A6): cuáles llegaron y si se abren aquí.
    let keys = signal(Vec::<ProviderKey>::new());
    let can_keys = signal(true);
    let fingerprint = signal(String::new());
    let slot: Slot = Arc::new(Mutex::new(None));
    // Para Ajustes (descartar lo que la nube rechazó).
    let slot_settings = slot.clone();
    let slot_mcp = slot.clone();
    // Vincular: el texto del QR (o solo el código) y la URL del deployment si hace falta.
    let pairing_text = signal(String::new());
    let pairing_url = signal(String::new());

    // Cerrar el hilo: primero se suelta el teclado y después se quita el campo; si se quita en el
    // mismo cambio, el «soltar» llega sin campo enfocado y el teclado queda abierto sobre la lista.
    let close_thread: std::rc::Rc<dyn Fn()> = std::rc::Rc::new(move || {
        dismiss_keyboard();
        app_owner.run(|| {
            spawn(async move {
                timer(Duration::from_millis(60)).await;
                thread_text.set(String::new());
                thread_open.set(None);
            })
        });
    });
    // El botón «atrás» de Android: cierra lo abierto (la cita, el hilo, el menú) antes de salir de la app.
    let close_back = close_thread.clone();
    on_back_press(move || {
        if global::<md::OpenCode>().get().0.is_some() {
            global::<md::OpenCode>().set(md::OpenCode(None));
        } else if thread_reply.get().is_some() {
            thread_reply.set(None);
        } else if thread_open.get().is_some() {
            close_back();
        } else if menu_open.get().is_some() || deploy_pick.get().is_some() {
            menu_open.set(None);
            deploy_pick.set(None);
        } else {
            return false;
        }
        true
    });

    // Los cuadros de lo que se anima en un hilo (el texto que llega, el brillo, el cursor): solo
    // mientras algo en pantalla los pide.
    spawn(async move {
        loop {
            timer(Duration::from_millis(40)).await;
            if live::wants_frame() {
                let t = global::<live::Tick>().get().0;
                global::<live::Tick>().set(live::Tick(t.wrapping_add(1)));
            } else {
                timer(Duration::from_millis(120)).await;
            }
        }
    });

    // Arranque y ciclo: identidad → (pendiente: esperar) → sincronizar cada 2 s.
    {
        let slot = slot.clone();
        spawn(async move {
            // Cuándo se preguntó si el teléfono sigue vinculado.
            let mut checked: Option<std::time::Instant> = None;
            // Fallas seguidas de la red: una sola (al volver de otra app la red tarda un momento) no
            // marca «Offline».
            let mut failures = 0u32;
            // Si este teléfono vio esperando su pedido de hilo nuevo (solo entonces se abre el que
            // creó la Mac; la respuesta de un pedido viejo no).
            let mut new_was_waiting = false;
            loop {
                let id = blocking(secure::load).await.ok().and_then(|r| r.ok()).flatten();
                let Some(id) = id else {
                    if screen.get() != Screen::Pair {
                        screen.set(Screen::Pair);
                    }
                    timer(Duration::from_secs(1)).await;
                    continue;
                };
                // De fondo, nada (ni red): Android no congela la app enseguida. Al volver, se pregunta y
                // se avisa de nuevo.
                if !blocking(scan::in_foreground).await.unwrap_or(true) {
                    if checked.take().is_some() {
                        with_phone(&slot, |p| p.pause()).await;
                    }
                    timer(Duration::from_secs(1)).await;
                    continue;
                }
                fingerprint.set(sync::fingerprint_of(&id));
                // Lo guardado primero (la réplica en disco): la lista al instante, antes de preguntar a la red.
                if slot.lock().map(|g| g.is_none()).unwrap_or(false) {
                    let opened = {
                        let id = id.clone();
                        let slot = slot.clone();
                        blocking(move || Phone::open(&id).map(|p| *slot.lock().unwrap() = Some(p))).await
                    };
                    if let Ok(Err(e)) = opened {
                        status.set(crate::i18n::tr(format!("Couldn't open: {e}"), format!("No se pudo abrir: {e}")));
                    }
                    if let Some((ths, incs, list, es, js)) = with_phone(&slot, |p| (p.threads(), p.incidents(), p.approvals(), p.envs(), p.jobs())).await
                        && (!ths.is_empty() || !incs.is_empty()) {
                            threads.set(ths);
                            incidents.set(incs);
                            approvals.set(list);
                            envs.set(es);
                            jobs.set(js);
                            loaded.set(true);
                            screen.set(Screen::Ready);
                        }
                }
                // Si sigue vinculado: al abrir y cada minuto (no en cada vuelta).
                let state = if checked.is_some_and(|t| t.elapsed() < Duration::from_secs(60)) && slot.lock().map(|g| g.is_some()).unwrap_or(false) {
                    Some(Ok("active".to_string()))
                } else {
                    let id = id.clone();
                    let s = blocking(move || sync::device_status(&id)).await.ok();
                    if matches!(s, Some(Ok(_))) {
                        checked = Some(std::time::Instant::now());
                    }
                    s
                };
                match state {
                    Some(Ok(s)) if s == "active" => {
                        if slot.lock().map(|g| g.is_none()).unwrap_or(false) {
                            let opened = {
                                let id = id.clone();
                                let slot = slot.clone();
                                blocking(move || Phone::open(&id).map(|p| *slot.lock().unwrap() = Some(p))).await
                            };
                            if let Ok(Err(e)) = opened {
                                status.set(crate::i18n::tr(format!("Couldn't open: {e}"), format!("No se pudo abrir: {e}")));
                            }
                        }
                        screen.set(Screen::Ready);
                        match with_phone(&slot, |p| p.sync().map(|list| (list, p.pending_changes(), p.incidents(), p.keys(), p.can_receive_keys(), p.threads(), p.mac_alive_at()))).await {
                            Some(Ok((list, pending, incs, ks, can, ths, alive))) => {
                                failures = 0;
                                // El hilo pedido desde un incidente: abrirlo cuando el daemon contesta.
                                if let Some(pid) = pending_thread.get()
                                    && let Some(i) = incs.iter().find(|i| i.id == pid && !i.waiting) {
                                        match i.result.as_deref() {
                                            Some(r) if r.starts_with("thread ") => {
                                                pending_thread.set(None);
                                                thread_text.set(String::new());
                                                thread_open.set(Some(r.trim_start_matches("thread ").to_string()));
                                                tab.set("threads");
                                            }
                                            Some(r) if r.starts_with("rejected") => pending_thread.set(None),
                                            _ => {}
                                        }
                                    }
                                incidents.set(incs);
                                if threads.get() != ths {
                                    threads.set(ths);
                                }
                                if !loaded.get() {
                                    loaded.set(true);
                                }
                                if let Some(cat) = with_phone(&slot, |p| p.catalog()).await
                                    && catalog.get() != cat {
                                        catalog.set(cat);
                                    }
                                if let Some(ps) = with_phone(&slot, |p| p.projects()).await
                                    && projects.get() != ps {
                                        projects.set(ps);
                                    }
                                if let Some((am, dm)) = with_phone(&slot, |p| (p.agent_models(), p.default_models())).await {
                                    if agent_models.get() != am {
                                        agent_models.set(am);
                                    }
                                    if default_models.get() != dm {
                                        default_models.set(dm);
                                    }
                                }
                                if let Some((cat, st)) = with_phone(&slot, |p| (p.run_catalog(), p.run_state())).await {
                                    if run_catalog.get() != cat {
                                        run_catalog.set(cat);
                                    }
                                    if run_state.get() != st {
                                        run_state.set(st);
                                    }
                                }
                                if let Some(so) = with_phone(&slot, |p| p.site_overview()).await
                                    && site_overview.get() != so {
                                        site_overview.set(so);
                                    }
                                if let Some((es, js, ov)) = with_phone(&slot, |p| (p.envs(), p.jobs(), p.overview())).await {
                                    if envs.get() != es {
                                        envs.set(es);
                                    }
                                    if overview.get() != ov {
                                        overview.set(ov);
                                    }
                                    if jobs.get() != js {
                                        jobs.set(js);
                                    }
                                }
                                if let Some(lang) = with_phone(&slot, |p| p.agent_language()).await
                                    && agent_lang.get() != lang {
                                        agent_lang.set(lang);
                                    }
                                if let Some(a) = with_phone(&slot, |p| p.latest_apk()).await
                                    && apk.get() != a {
                                        apk.set(a);
                                    }
                                if let Some(nt) = with_phone(&slot, |p| p.new_thread()).await {
                                    // El daemon creó el hilo pedido desde «New thread»: abrirlo.
                                    if nt.waiting {
                                        new_was_waiting = true;
                                    }
                                    if new_was_waiting
                                        && thread_open.get().as_deref() == Some("_new")
                                        && let Some(id) = nt.result.as_deref().and_then(|r| r.strip_prefix("thread ")) {
                                            thread_open.set(Some(id.to_string()));
                                            new_was_waiting = false;
                                        }
                                    if new_thread.get().as_ref() != Some(&nt) {
                                        new_thread.set(Some(nt));
                                    }
                                }
                                if keys.get() != ks {
                                    keys.set(ks);
                                }
                                can_keys.set(can);
                                let waiting = list.iter().filter(|a| a.status == "waiting-approval").count();
                                waiting_count.set(waiting);
                                if status.get().starts_with("Offline") {
                                    status.set(String::new());
                                }
                                // Sin latido de la Mac en 3 min (late cada minuto) o despedida: está apagada
                                // o sin red; contesta el sitio en la nube, si hay.
                                let mac_off = alive.filter(|at| sync::now_ms() - at > MAC_OFFLINE_MS);
                                let p = if mac_off.is_some() { chat::Presence::Cloud } else { chat::Presence::Mac };
                                if presence.get() != p {
                                    presence.set(p);
                                }
                                if let Some(ob) = with_phone(&slot, |p| (p.pending_changes(), p.recovery(), p.sync_debug())).await
                                    && outbox.get() != ob {
                                        outbox.set(ob);
                                    }
                                sync_line.set(match (pending, mac_off) {
                                    (0, Some(0)) => "Mac offline".to_string(),
                                    (0, Some(at)) => crate::i18n::tr(format!("Mac offline · {}", theme::ago(at)), format!("Mac apagada · {}", theme::ago(at))),
                                    (0, None) => "in sync".to_string(),
                                    (n, _) => format!("sending {n}…"),
                                });
                                if approvals.get() != list {
                                    approvals.set(list);
                                }
                            }
                            Some(Err(e)) => {
                                failures += 1;
                                if failures >= 2 {
                                    presence.set(chat::Presence::Offline);
                                    sync_line.set("offline".into());
                                    status.set(format!("Offline: {}", e.chars().take(80).collect::<String>()));
                                }
                            }
                            None => {}
                        }
                    }
                    Some(Ok(s)) if s == "pending" => screen.set(Screen::Waiting(sync::fingerprint_of(&id))),
                    Some(Ok(_)) => {
                        // Revocado (o desconocido): se olvida la identidad y se vuelve a vincular.
                        let _ = blocking(secure::clear).await;
                        *slot.lock().unwrap() = None;
                        status.set("This phone was removed on the Mac. Pair it again.".into());
                        screen.set(Screen::Pair);
                    }
                    Some(Err(e)) => status.set(format!("Offline: {}", e.chars().take(80).collect::<String>())),
                    None => {}
                }
                tick.set(tick.get().wrapping_add(1));
                // Enseguida al mandar algo (`kick`).
                let busy = threads.get().iter().any(|t| t.waiting || t.items.iter().any(|i| i["live"] == true)) || incidents.get().iter().any(|i| i.waiting) || approvals.get().iter().any(|a| a.pending) || pending_thread.get().is_some();
                // Con el timbre en vivo se espera a que suene (o a mandar algo), hasta un minuto; sin él,
                // cada 4 s (cada segundo mientras algo espera respuesta).
                let live = with_phone(&slot, |p| p.bell().1).await.unwrap_or(false);
                let steps = if live { 240 } else if busy { 4 } else { 16 };
                for _ in 0..steps {
                    if kick.get() || (live && with_phone(&slot, |p| p.bell().0).await.unwrap_or(false)) {
                        break;
                    }
                    timer(Duration::from_millis(250)).await;
                }
                kick.set(false);
            }
        });
    }

    let do_pair = move || {
        let (text_v, url_v) = (pairing_text.get(), pairing_url.get());
        spawn(async move {
            status.set("Pairing…".into());
            let result = blocking(move || {
                let (url, code) = sync::parse_pairing(&text_v, &url_v)?;
                let id = sync::pair(&url, &code, DEVICE_NAME)?;
                secure::save(&id)?;
                Ok::<_, String>(sync::fingerprint_of(&id))
            })
            .await;
            match result {
                Ok(Ok(fp)) => {
                    status.set(String::new());
                    screen.set(Screen::Waiting(fp));
                }
                Ok(Err(e)) => status.set(e),
                Err(_) => {}
            }
        });
    };

    // Escanear el QR de la Mac: la cámara del sistema saca la foto y aquí se lee (scan.rs). Si
    // Android cerró la app mientras la cámara estaba abierta, el escaneo sigue al volver.
    let scanning = signal(false);
    let watch_scan = move || {
        spawn(async move {
            scanning.set(true);
            status.set("Take a photo of the QR on the Mac…".into());
            for _ in 0..180 {
                timer(Duration::from_secs(1)).await;
                if !scanning.get() {
                    return;
                }
                match blocking(scan::poll).await {
                    Ok(scan::Poll::Waiting) => continue,
                    Ok(scan::Poll::Done(text)) => {
                        scanning.set(false);
                        pairing_text.set(text);
                        status.set(String::new());
                        return do_pair();
                    }
                    Ok(scan::Poll::Failed(e)) => {
                        scanning.set(false);
                        return status.set(e);
                    }
                    Err(_) => break,
                }
            }
            let _ = blocking(scan::cancel).await;
            scanning.set(false);
            status.set("No photo arrived. Try again, or use “Paste the QR's text”.".into());
        });
    };
    let scan_qr = move || {
        spawn(async move {
            match blocking(scan::start).await {
                Ok(Ok(())) => watch_scan(),
                // Sin la foto de vuelta: la cámara sola, cuyo lector de QR deja copiar el texto.
                Ok(Err(e)) => match blocking(scan::open_camera_app).await {
                    Ok(Ok(())) => status.set(format!("Couldn't get the photo back ({e}). In the camera, point at the QR, copy the text it shows, come back and tap “Paste the QR's text”.")),
                    _ => status.set(format!("{e}. Scan the QR with your camera or Lens, copy the text and tap “Paste the QR's text”.")),
                },
                Err(_) => {}
            }
        });
    };
    let cancel_scan = move || {
        scanning.set(false);
        status.set(String::new());
        spawn(async move {
            let _ = blocking(scan::cancel).await;
        });
    };
    // La cámara del teléfono (o Lens) lee el QR y copia su texto; aquí se pega y se vincula.
    let paste_qr = move || {
        spawn(async move {
            match blocking(scan::clipboard).await {
                Ok(Ok(text)) => {
                    pairing_text.set(text);
                    do_pair();
                }
                Ok(Err(e)) => status.set(e),
                Err(_) => {}
            }
        });
    };
    if scan::supported() && scan::pending() {
        watch_scan();
    }

    let decide = {
        let slot = slot.clone();
        move |job: String, approve: bool, confirm: Option<String>| {
            let slot = slot.clone();
            spawn(async move {
                if let Some(Err(e)) = with_phone(&slot, move |p| p.decide(&job, approve, confirm)).await {
                    status.set(crate::i18n::tr(format!("Couldn't decide: {e}"), format!("No se pudo decidir: {e}")));
                }
                kick.set(true);
                if let Some(list) = with_phone(&slot, |p| p.approvals()).await {
                    approvals.set(list);
                }
            });
        }
    };

    // La misma decisión, para las tarjetas de aprobación dentro de los hilos.
    let decide_rc: chat::Decide = {
        let d = decide.clone();
        std::rc::Rc::new(d)
    };

    // Un pedido sobre un incidente: sale firmado en el próximo sync; el daemon decide.
    // Pedir correr algo (firmado en `_run`): sale enseguida y se ve qué contestó la Mac.
    let run_send: std::rc::Rc<dyn Fn(serde_json::Value)> = {
        let slot = slot.clone();
        std::rc::Rc::new(move |ask: serde_json::Value| {
            let slot = slot.clone();
            spawn(async move {
                if let Some(Err(e)) = with_phone(&slot, move |p| p.run(&ask)).await {
                    status.set(crate::i18n::tr(format!("Couldn't send: {e}"), format!("No se pudo enviar: {e}")));
                }
                kick.set(true);
                if let Some(st) = with_phone(&slot, |p| p.run_state()).await {
                    run_state.set(st);
                }
            });
        })
    };
    let ask = {
        let slot = slot.clone();
        move |id: String, request: &'static str, arg: String| {
            let slot = slot.clone();
            spawn(async move {
                if let Some(Err(e)) = with_phone(&slot, move |p| p.request(&id, request, &arg)).await {
                    status.set(crate::i18n::tr(format!("Couldn't send: {e}"), format!("No se pudo enviar: {e}")));
                }
                kick.set(true);
                if let Some(list) = with_phone(&slot, |p| p.incidents()).await {
                    incidents.set(list);
                }
            });
        }
    };

    // Escribir en un hilo: sale firmado en el próximo sync; el daemon decide a quién va.
    let send_thread = {
        let slot = slot.clone();
        move |id: String, text: String| {
            let slot = slot.clone();
            // Los adjuntos ya subidos van con el mensaje; el campo queda vacío.
            let files: Vec<sync::Attachment> = thread_files
                .get()
                .into_iter()
                .filter_map(|f| match f.uploaded {
                    Some(Ok(id)) => Some(sync::Attachment { id, name: f.name, mime: f.mime, kind: f.kind, seconds: f.seconds }),
                    _ => None,
                })
                .collect();
            // Solo si había: vaciar una lista vacía vuelve a dibujar el campo y le quita el foco.
            if !thread_files.get().is_empty() {
                thread_files.set(vec![]);
            }
            last_send.set(format!("{} · {} chars → {id}", theme::short(sync::now_ms()), text.chars().count()));
            let sent = app_owner.run(|| spawn(async move {
                let tid = id.clone();
                match with_phone(&slot, move |p| p.thread_send_with(&id, &text, &files)).await {
                    Some(Ok(())) => last_send.set(format!("ok → {tid}")),
                    Some(Err(e)) => {
                        last_send.set(format!("error → {tid}: {e}"));
                        status.set(crate::i18n::tr(format!("Couldn't send: {e}"), format!("No se pudo enviar: {e}")));
                    }
                    None => last_send.set(format!("no phone → {tid}")),
                }
                kick.set(true);
                if let Some(list) = with_phone(&slot, |p| p.threads()).await {
                    threads.set(list);
                }
            }));
            let _ = sent;
        }
    };

    // Adjuntar: cada archivo se cifra para la Mac y sube enseguida; el mensaje lleva los ya subidos.
    let next_local = Arc::new(std::sync::atomic::AtomicU64::new(1));
    let add_file = {
        let slot = slot.clone();
        let next_local = next_local.clone();
        move |bytes: Vec<u8>, name: String, mime: String, kind: &'static str, seconds: Option<f64>, then: Option<Box<dyn FnOnce()>>| {
            let local = next_local.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut list = thread_files.get();
            list.push(chat::PendingFile { local, name, mime, kind: kind.into(), seconds, uploaded: None });
            thread_files.set(list);
            let slot = slot.clone();
            spawn(async move {
                let res = with_phone(&slot, move |p| p.upload(&bytes)).await.unwrap_or_else(|| Err("not paired".into()));
                let ok = res.is_ok();
                thread_files.set(thread_files.get().into_iter().map(|mut f| {
                    if f.local == local {
                        f.uploaded = Some(res.clone());
                    }
                    f
                }).collect());
                if ok
                    && let Some(then) = then {
                        then();
                    }
            });
        }
    };
    let composer = {
        let add_photo = add_file.clone();
        let add_voice = add_file.clone();
        let send_voice = send_thread.clone();
        chat::Composer {
            files: thread_files,
            recording,
            menu: attach_menu,
            approvals,
            decide: decide_rc.clone(),
            reply: thread_reply,
            catalog,
            close: close_thread.clone(),
            presence,
            agent_models,
            default_models,
            model_pick: signal(None),
            model_menu: signal(false),
            projects,
            run: run_send.clone(),
            run_state,
            // El último proyecto elegido sigue elegido (se cambia en el «+»).
            project_pick: signal(theme::pref("lastProject").as_str().map(str::to_string)),
            photo: std::rc::Rc::new(move || {
                let add = add_photo.clone();
                app_owner.run(|| spawn(async move {
                    match blocking(scan::start_photo).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => return status.set(e),
                        Err(_) => return,
                    }
                    for _ in 0..240 {
                        timer(Duration::from_secs(1)).await;
                        match blocking(scan::poll_photo).await {
                            Ok(scan::PhotoPoll::Waiting) => continue,
                            Ok(scan::PhotoPoll::Done(bytes)) => {
                                match blocking(move || scan::shrink_photo(&bytes)).await {
                                    Ok(Ok(jpeg)) => add(jpeg, format!("photo-{}.jpg", sync::now_ms() / 1000 % 100_000), "image/jpeg".into(), "image", None, None),
                                    Ok(Err(e)) => status.set(e),
                                    Err(_) => {}
                                }
                                return;
                            }
                            Ok(scan::PhotoPoll::Failed(e)) => return status.set(e),
                            Err(_) => return,
                        }
                    }
                }));
            }),
            mic: std::rc::Rc::new(move || {
                if audio::permission() != audio::Permission::Granted {
                    if let Err(e) = audio::request_permission() {
                        status.set(e);
                    }
                    status.set("Allow the microphone for Condx, then tap the microphone again.".into());
                    return;
                }
                if let Err(e) = audio::start() {
                    return status.set(e);
                }
                status.set(String::new());
                recording.set(Some(0.0));
                // El reloj de la grabación (y el corte a los 3 minutos).
                app_owner.run(|| spawn(async move {
                    while recording.get().is_some() {
                        timer(Duration::from_millis(250)).await;
                        match audio::elapsed() {
                            Some(s) if s >= audio::MAX_SECONDS => {
                                recording.set(Some(s));
                                break;
                            }
                            Some(s) => recording.set(Some(s)),
                            None => break,
                        }
                    }
                }));
            }),
            stop: std::rc::Rc::new(move |send: bool| {
                recording.set(None);
                if !send {
                    audio::cancel();
                    return;
                }
                let add = add_voice.clone();
                let send_voice = send_voice.clone();
                app_owner.run(|| spawn(async move {
                    match blocking(audio::stop).await {
                        Ok(Ok(rec)) => {
                            // La nota de voz se manda sola al terminar de subir (como en WhatsApp).
                            let then: Box<dyn FnOnce()> = Box::new(move || {
                                if let Some(id) = thread_open.get() {
                                    let text = thread_text.get();
                                    thread_text.set(String::new());
                                    send_voice(id, text);
                                }
                            });
                            add(rec.bytes, "voice.m4a".into(), "audio/mp4".into(), "audio", Some(rec.seconds), Some(then));
                        }
                        Ok(Err(e)) => status.set(e),
                        Err(_) => {}
                    }
                }));
            }),
        }
    };

    // El hilo de un incidente: el del agente que ya trabajó en él, o el que abre el daemon.
    let open_incident_thread = {
        let ask = ask.clone();
        move |id: String, run: Option<String>| match run {
            Some(r) => {
                thread_text.set(String::new());
                thread_open.set(Some(r));
                tab.set("threads");
            }
            None => {
                pending_thread.set(Some(id.clone()));
                ask(id, "thread", String::new());
            }
        }
    };

    let unpair = {
        let slot = slot.clone();
        move || {
            let slot = slot.clone();
            spawn(async move {
                let _ = blocking(secure::clear).await;
                *slot.lock().unwrap() = None;
                approvals.set(vec![]);
                screen.set(Screen::Pair);
            });
        }
    };

    // Como en la app de escritorio, solo lo que espera; y lo que decidió este teléfono, para ver
    // qué hizo la Mac con eso.
    // Lo que espera primero; lo decidido aquí solo del último día (si no, el historial tapa lo que importa).
    let visible = move || {
        let day = sync::now_ms() - 24 * 3600 * 1000;
        // Las que la Mac rechazó por viejas («rejected: stale decision») ya no esperan nada: se ocultan.
        let stale = |a: &Approval| a.status != "waiting-approval" && a.phone_result.as_deref().is_some_and(|r| r.starts_with("rejected: "));
        let mut list: Vec<Approval> = approvals.get().into_iter().filter(|a| !stale(a) && (a.status == "waiting-approval" || (a.decision.is_some() && a.created_at > day))).collect();
        list.sort_by_key(|a| (a.status != "waiting-approval", std::cmp::Reverse(a.created_at)));
        list
    };

    let row = move |a: ReadSignal<Approval>, _ix: ReadSignal<usize>| {
        let (d1, d2) = (decide.clone(), decide.clone());
        let item = a.get();
        let (job_a, job_r) = (item.job_id.clone(), item.job_id.clone());
        let env = item.env.clone();
        let typed = item.level == "typed";
        // Producción (o aprobación reforzada): el teléfono solo propone; la Mac confirma con Touch ID.
        let prod = item.level == "reinforced" || item.env_kind == "cloud-prod";
        let waiting = item.status == "waiting-approval" && item.decision.is_none();
        let (health, state) = match (&item.decision, item.pending, item.phone_result.as_deref(), item.status.as_str()) {
            (Some(_), true, _, _) => (Health::Unknown, "Sending your decision…".to_string()),
            (Some(_), false, Some("approved"), _) => (Health::Ok, "Approved from this phone".to_string()),
            (Some(_), false, Some("rejected the job"), _) => (Health::Unknown, "Rejected from this phone".to_string()),
            (Some(_), false, Some("needs-mac"), _) => (Health::Warn, "Waiting for Touch ID on the Mac".to_string()),
            // Llegó tarde o la Mac la rechazó: un resultado, no una alarma.
            (Some(_), false, Some(r), _) => (Health::Unknown, crate::i18n::tr(format!("Not applied: {}", r.trim_start_matches("rejected: ")), format!("No se aplicó: {}", r.trim_start_matches("rejected: ")))),
            (Some(d), false, None, _) => (Health::Unknown, crate::i18n::tr(format!("You chose {d} · the Mac checks it"), format!("Elegiste {d} · la Mac lo revisa"))),
            (None, _, _, "waiting-approval") => (Health::Warn, if prod { "Production: Touch ID on the Mac".to_string() } else { crate::i18n::tr(format!("Needs your approval · {}", item.level), format!("Espera tu aprobación · {}", item.level)) }),
            (None, _, _, s) => (Health::Unknown, crate::i18n::tr(format!("Done: {s}"), format!("Terminado: {s}"))),
        };
        let (requested_by, recipe, created_at) = (item.requested_by.clone(), item.recipe.clone(), item.created_at);
        let summary = move || {
            tick.get();
            // «workflow “Nombre”» → «flujo “Nombre”» (el nombre no se traduce).
            let who = match requested_by.strip_prefix("workflow ") {
                Some(name) => format!("{} {name}", crate::i18n::t("workflow")),
                None => crate::i18n::t(&requested_by),
            };
            format!("{recipe} · {} {who}{}", crate::i18n::t("by"), if created_at > 0 { format!(" · {}", theme::ago(created_at)) } else { String::new() })
        };
        view()
            .gap(px(6.0))
            .px_3()
            .py(px(10.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(c(tokens::CARD_BORDER))
            .bg(c(tokens::CARD))
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(theme::dot(health))
                    .child(view().flex_1().child(text(format!("{} · {}", item.title, item.env)).number_of_lines(1u32).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))))
                    .child(kind_tag(&item.env_kind)),
            )
            .child(text(summary).number_of_lines(1u32).text_size(px(12.0)).text_color(c(tokens::MUTED)))
            .child(text(state).text_size(px(12.0)).text_color(c(if health == Health::Warn { tokens::WARNING } else { tokens::MUTED })))
            .child(match (waiting, typed && !prod) {
                // «typed»: en la Mac se escribe el nombre del entorno; aquí se desliza hasta el final
                // (igual de deliberado) y la decisión lleva el nombre que el daemon compara.
                (true, true) => Some(
                    view()
                        .gap_2()
                        .pt_1()
                        .child(theme::slide_to_confirm("Slide to approve", move || d1(job_a.clone(), true, Some(env.clone()))))
                        .child(view().flex_row().justify_end().child(theme::small_button("Reject", Variant::Danger, move || d2(job_r.clone(), false, None))))
                        .into_element(),
                ),
                (true, false) => Some(
                    view()
                        .flex_row()
                        .gap_2()
                        .pt_1()
                        .justify_end()
                        .child(theme::small_button("Reject", Variant::Danger, move || d2(job_r.clone(), false, None)))
                        .child(theme::small_button(if prod { "Propose" } else { "Approve" }, Variant::Primary, move || d1(job_a.clone(), true, None)))
                        .into_element(),
                ),
                (false, _) => None,
            })
    };

    // Un incidente: una fila compacta (estado, título, origen, cuántas veces, cuándo) que se abre al
    // tocarla con el error, lo que hizo el agente y las acciones (las de menos uso en «⋯»).
    let incident_row = move |i: ReadSignal<Incident>, _ix: ReadSignal<usize>| {
        let inc = i.get();
        let agent = inc.agent.clone();
        let agent_status = agent["status"].as_str().unwrap_or("").to_string();
        let running = agent_status == "running";
        let open = inc.status != "resolved";
        let fix = agent["fix"].clone();
        let fix_ready = fix["changes"] == true && fix["discarded"] != true && fix["commit"].is_string();
        let is_expanded = expanded.get().contains(&inc.id);
        let menu = menu_open.get().as_deref() == Some(inc.id.as_str());
        let health = match inc.status.as_str() {
            "resolved" => Health::Ok,
            "ignored" => Health::Unknown,
            _ => Health::Bad,
        };
        let agent_line = agent.is_object().then(|| match (agent_status.as_str(), agent["mode"].as_str()) {
            ("running", Some("fix")) => (Health::Warn, "Preparing a fix…".to_string()),
            ("running", _) => (Health::Warn, "Diagnosing…".to_string()),
            ("done", Some("fix")) if fix_ready => (Health::Ok, "Fix ready".to_string()),
            ("done", Some("fix")) => (Health::Ok, "Fix attempt done".to_string()),
            ("done", _) => (Health::Ok, "Diagnosis ready".to_string()),
            ("stopped", _) => (Health::Unknown, "Agent stopped (limit or budget)".to_string()),
            ("aborted", _) => (Health::Unknown, "Agent stopped".to_string()),
            _ => (Health::Bad, "Agent failed".to_string()),
        });
        let meta = [Some(inc.source.replace('-', " ")), (!inc.env.is_empty()).then(|| inc.env.clone()), (inc.count > 1).then(|| format!("{}×", inc.count)), (inc.status != "open").then(|| inc.status.clone())];
        let id_t = inc.id.clone();
        let mut k = view()
            .gap(px(6.0))
            .px_3()
            .py(px(10.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(c(tokens::CARD_BORDER))
            .bg(c(tokens::CARD))
            .child(
                view()
                    .gap(px(4.0))
                    .on_press(move || {
                        let mut e = expanded.get();
                        if let Some(ix) = e.iter().position(|x| x == &id_t) {
                            e.remove(ix);
                        } else {
                            e.push(id_t.clone());
                        }
                        expanded.set(e);
                    })
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .child(theme::dot(health))
                            .child(view().flex_1().child(text(inc.title.clone()).number_of_lines(if is_expanded { 3u32 } else { 1u32 }).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))))
                            .child(text(theme::short(inc.last_seen)).text_size(px(11.5)).text_color(c(tokens::SUBTLE))),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .pl(px(15.0))
                            .child(text(meta.into_iter().flatten().collect::<Vec<_>>().join(" · ")).number_of_lines(1u32).text_size(px(12.0)).text_color(c(tokens::MUTED)))
                            .child(match &agent_line {
                                Some((h, l)) => view().flex_row().gap(px(5.0)).items_center().child(text("·").text_size(px(12.0)).text_color(c(tokens::SUBTLE))).child(theme::dot(*h)).child(text(l.clone()).number_of_lines(1u32).text_size(px(12.0)).text_color(c(tokens::MUTED))).into_element(),
                                None => view().into_element(),
                            }),
                    ),
            );
        // Lo que hizo el daemon con el último pedido de este teléfono (se ve aunque esté cerrada).
        if inc.waiting {
            k = k.child(view().pl(px(15.0)).child(status_tag(Health::Unknown, "Sending your request…")));
        } else if let Some(r) = inc.result.as_ref().filter(|r| !r.starts_with("thread ")) {
            k = k.child(view().pl(px(15.0)).child(status_tag(if r.starts_with("rejected") { Health::Bad } else { Health::Ok }, r.clone())));
            // «PR <url> · workflow …»: el PR que se abrió antes de desplegar, para verlo.
            if let Some(url) = r.strip_prefix("PR ").and_then(|t| t.split_whitespace().next()).filter(|u| u.starts_with("https://")) {
                let url = url.to_string();
                k = k.child(view().pl(px(15.0)).flex_row().child(theme::small_button("Open the PR", Variant::Outline, move || {
                    let _ = scan::open_url(&url);
                })));
            }
        }
        if !is_expanded {
            return k;
        }
        if !inc.error.is_empty() {
            k = k.child(
                view()
                    .px_2()
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(c(tokens::HOVER))
                    .child(text(inc.error.lines().take(6).collect::<Vec<_>>().join("\n")).number_of_lines(6u32).font_family(theme::MONO_FONT).text_size(px(11.5)).line_height(px(16.0)).text_color(c(tokens::PROD))),
            );
        }
        // La respuesta del agente: la causa; toda al pedirla.
        if let Some(answer) = agent["answer"].as_str().filter(|a| !a.trim().is_empty()) {
            let whole = answer_open.get().contains(&inc.id);
            let shown: String = if whole {
                answer.to_string()
            } else {
                answer.split("## ").find(|s| s.starts_with("Cause")).map(|s| s.trim_start_matches("Cause").trim().to_string()).unwrap_or_else(|| answer.chars().take(240).collect())
            };
            let col = md::render(&shown, 13.0);
            let id = inc.id.clone();
            k = k.child(col).child(text(if whole { "Show less" } else { "Show the whole answer" }).text_size(px(12.0)).text_color(c(tokens::MUTED)).on_press(move || {
                let mut e = answer_open.get();
                if let Some(ix) = e.iter().position(|x| x == &id) {
                    e.remove(ix);
                } else {
                    e.push(id.clone());
                }
                answer_open.set(e);
            }));
        }
        if fix_ready {
            k = k.child(status_tag(Health::Ok, {
                let n = fix["files"].as_array().map(Vec::len).unwrap_or(0);
                crate::i18n::tr(format!("Fix on {} · {n} file{}", fix["branch"].as_str().unwrap_or("?"), if n == 1 { "" } else { "s" }), format!("Arreglo en {} · {n} archivo{}", fix["branch"].as_str().unwrap_or("?"), if n == 1 { "" } else { "s" }))
            }));
        }
        // Acciones: las de siempre como botones chicos; las demás en «⋯».
        let (open_thread, id_th) = (open_incident_thread.clone(), inc.id.clone());
        let agent_run = agent["runId"].as_str().map(str::to_string);
        let mut actions = view().flex_row().gap_2().pt_1().flex_wrap().items_center();
        if open && !inc.waiting && !running {
            let (a1, a2, id1, id2) = (ask.clone(), ask.clone(), inc.id.clone(), inc.id.clone());
            actions = actions
                .child(theme::small_button(if agent.is_object() { "Diagnose again" } else { "Diagnose" }, Variant::Primary, move || a1(id1.clone(), "diagnose", String::new())))
                .child(theme::small_button("Fix", Variant::Outline, move || a2(id2.clone(), "fix", String::new())));
        }
        actions = actions.child(theme::small_button("Thread", Variant::Outline, move || open_thread(id_th.clone(), agent_run.clone())));
        let id_m = inc.id.clone();
        actions = actions.child(view().flex_1()).child(theme::small_button("⋯", Variant::Outline, move || menu_open.set(if menu_open.get().as_deref() == Some(id_m.as_str()) { None } else { Some(id_m.clone()) })));
        k = k.child(actions);
        if menu {
            let mut m = view().rounded(px(8.0)).border_1().border_color(c(tokens::CARD_BORDER)).overflow_hidden();
            let mut entries: Vec<(&'static str, &'static str, bool)> = Vec::new();
            if open && !inc.waiting {
                entries.push(("Ignore", "ignore", false));
                entries.push(("Resolve", "resolve", false));
            }
            if fix_ready {
                entries.push(("Discard the fix", "discard", true));
            }
            // Desplegar el arreglo con un flujo encendido: se elige aquí y se confirma deslizando.
            let deploys: Vec<(String, String)> = if fix_ready && open && !inc.waiting && !running { inc.workflows.iter().take(4).cloned().collect() } else { vec![] };
            if entries.is_empty() && deploys.is_empty() {
                m = m.child(view().px_3().py_2().child(muted("Nothing else to do here.")));
            }
            let n = entries.len();
            for (ix, (label, req, danger)) in entries.into_iter().enumerate() {
                let (a, id) = (ask.clone(), inc.id.clone());
                let mut item = view()
                    .px_3()
                    .h(px(40.0))
                    .justify_center()
                    .active(|s| s.bg(c(tokens::ACTIVE)))
                    .on_press(move || {
                        menu_open.set(None);
                        a(id.clone(), req, String::new());
                    })
                    .child(text(label).text_size(px(13.5)).text_color(c(if danger { tokens::DANGER } else { tokens::FG })));
                if ix + 1 < n {
                    item = item.border_b_1().border_color(c(tokens::CARD_BORDER));
                }
                m = m.child(item);
            }
            for (wid, name) in deploys {
                let pick = (inc.id.clone(), wid, name.clone());
                m = m.child(
                    view()
                        .px_3()
                        .h(px(40.0))
                        .justify_center()
                        .border_t_1()
                        .border_color(c(tokens::CARD_BORDER))
                        .active(|s| s.bg(c(tokens::ACTIVE)))
                        .on_press(move || {
                            menu_open.set(None);
                            deploy_pick.set(Some(pick.clone()));
                        })
                        .child(text(crate::i18n::tr(format!("Deploy the fix with “{name}”"), format!("Desplegar el arreglo con «{name}»"))).number_of_lines(1u32).text_size(px(13.5)).text_color(c(tokens::FG))),
                );
            }
            k = k.child(m);
        }
        // Desplegar: el flujo elegido en «⋯», confirmado deslizando (el flujo pide sus propias aprobaciones).
        if let Some((_, wid, name)) = deploy_pick.get().filter(|d| d.0 == inc.id) {
            let (a, id) = (ask.clone(), inc.id.clone());
            let direct = deploy_direct.get();
            k = k
                .child(theme::slide_to_confirm(crate::i18n::tr(format!("Slide to run “{name}”"), format!("Desliza para correr «{name}»")), move || {
                    deploy_pick.set(None);
                    deploy_direct.set(false);
                    a(id.clone(), "deploy", if direct { format!("{wid}|direct") } else { wid.clone() });
                }))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(view().flex_1().child(muted(if direct { "Deploys directly, without a PR." } else { "Opens a PR with the fix first." })))
                        .child(theme::small_button(if direct { "With a PR" } else { "No PR" }, Variant::Outline, move || deploy_direct.set(!direct)))
                        .child(theme::small_button("Cancel", Variant::Outline, move || {
                            deploy_pick.set(None);
                            deploy_direct.set(false);
                        })),
                );
        } else if fix_ready && open && inc.workflows.is_empty() {
            k = k.child(muted("Turn on a workflow on the Mac to deploy fixes from here."));
        }
        k
    };

    let pair_view = move || {
        view()
            .gap_3()
            .child(section("Pair with your Mac"))
            .child(
                card()
                    .gap_3()
                    .child(muted(if scan::supported() {
                        "On the Mac: Condx → Settings → Condx Cloud → Pair a phone. Scan its QR here, or with your phone's camera: it opens a page with a Copy button, then tap “Paste the QR's text”."
                    } else {
                        "On the Mac: Condx → Settings → Condx Cloud → Pair a phone. Paste the QR's text, or type the code and the deployment URL."
                    }))
                    .child(dynamic(move || {
                        if !scan::supported() {
                            view().into_element()
                        } else if scanning.get() {
                            view().flex_row().child(theme::button("Cancel the scan", Variant::Outline, cancel_scan)).into_element()
                        } else {
                            view()
                                .gap_2()
                                .child(view().flex_row().child(theme::button("Scan the QR", Variant::Primary, scan_qr)))
                                .child(view().flex_row().child(theme::button("Paste the QR's text", Variant::Outline, paste_qr)))
                                .into_element()
                        }
                    }))
                    .child(field("Code (K7Q2-M9XD-4HWR) or the QR's text", pairing_text).auto_capitalize(AutoCapitalize::None))
                    .child(field("Deployment URL (only with a typed code)", pairing_url).auto_capitalize(AutoCapitalize::None).keyboard_type(KeyboardType::Url))
                    .child(view().flex_row().child(theme::button("Pair", if scan::supported() { Variant::Outline } else { Variant::Primary }, do_pair))),
            )
    };

    // Barra superior, como la de la app de escritorio: la marca a la izquierda; lo que espera y
    // el estado de la sincronización a la derecha.
    let top_bar = move || {
        view()
            .flex_row()
            .items_center()
            .gap_3()
            .px_4()
            .h(px(44.0))
            .border_b_1()
            .border_color(c(tokens::CARD_BORDER))
            .bg(c(tokens::GROUND))
            // La marca de Condx al lado del nombre.
            .child(theme::icon(theme::CONDX_MARK, tokens::ACCENT, 20.0))
            .child(text("Condx").text_size(px(15.0)).font_weight(FontWeight::MEDIUM))
            .child(view().flex_1())
            .child(dynamic(move || {
                let n = waiting_count.get();
                if screen.get() == Screen::Ready && n > 0 {
                    view()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .px_2()
                        .h(px(24.0))
                        .border_1()
                        .border_color(c(tokens::CARD_BORDER))
                        .child(theme::dot(Health::Warn))
                        .child(text(crate::i18n::tr(format!("{n} to approve"), format!("{n} por aprobar"))).text_size(px(12.0)))
                        .into_element()
                } else {
                    view().into_element()
                }
            }))
            // Quién contesta (laptop verde: la Mac; nube azul: el sitio en la nube; nube roja: sin
            // conexión), como en el cuadro de escribir, y los ajustes.
            .child(dynamic(move || if screen.get() == Screen::Ready { chat::presence_icon(presence.get()) } else { view().into_element() }))
            .child(dynamic(move || {
                if screen.get() != Screen::Ready {
                    return view().into_element();
                }
                view()
                    .w(px(32.0))
                    .h(px(32.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(16.0))
                    .active(|s| s.bg(c(tokens::ACTIVE)))
                    .on_press(move || {
                        thread_open.set(None);
                        tab.set(if tab.get() == "settings" { "threads" } else { "settings" });
                    })
                    .child(theme::icon(theme::SLIDERS, if tab.get() == "settings" { tokens::FG } else { tokens::MUTED }, 18.0))
                    .into_element()
            }))
    };

    // La barra de estado sigue al tema elegido (íconos oscuros en el claro), no al del sistema.
    let bar = move || nsh_core::StatusBarSetting {
        style: Some(match scheme.get().as_str() {
            "light" => nsh_core::StatusBarStyle::Dark,
            "dark" => nsh_core::StatusBarStyle::Light,
            _ => nsh_core::StatusBarStyle::Default,
        }),
        ..Default::default()
    };
    #[allow(deprecated)]
    let (shell, page) = (nsh_core::ui::stack(), nsh_core::ui::screen());
    shell.flex_1().child(page.flex_1().header_visible(false).status_bar(bar).child(view()
        .flex_1()
        .bg(c(tokens::GROUND))
        .text_color(c(tokens::FG))
        .font_family(theme::UI_FONT)
        .text_size(px(14.0))
        .child(
            safe_area_view().flex_1().child(
                view()
                    .flex_1()
                    // El código a pantalla completa, encima de todo (`md::fullscreen`).
                    .child(md::fullscreen())
                    // Dentro de un hilo la barra no ocupa lugar (el hilo tiene su cabecera); solo
                    // aparece una línea si la sincronización no anda.
                    .child(dynamic(move || {
                        if thread_open.get().is_none() {
                            return top_bar().into_element();
                        }
                        // Dentro de un hilo no hay franja: quién contesta va en la esquina del cuadro de escribir.
                        view().into_element()
                    }))
                    .child(
                        view()
                            .flex_1()
                            .bg(c(tokens::PANEL))
                            .child(dynamic(move || {
                                let notice = status.get();
                                if notice.is_empty() { view().into_element() } else { view().px_4().pt_3().child(status_tag(Health::Bad, notice)).into_element() }
                            }))
                            .child(dynamic(move || match screen.get() {
                                Screen::Loading => view().p_4().child(muted("…")).into_element(),
                                Screen::Pair => view().p_4().child(pair_view()).into_element(),
                                Screen::Waiting(fp) => view()
                                    .p_4()
                                    .gap_3()
                                    .child(section("Confirm on your Mac"))
                                    .child(
                                        card()
                                            .gap_3()
                                            .child(muted("Check that the Mac shows this same key, then press Confirm there."))
                                            .child(mono(fp, 22.0))
                                            .child(status_tag(Health::Warn, "Waiting for the Mac…")),
                                    )
                                    .into_element(),
                                Screen::Ready => view()
                                    .flex_1()
                                    .child(scroll()
                                    .flex_1()
                                    .element_ref(&main_scroll)
                                    // Cerca del final, 20 hilos más (la lista entera no se arma de una vez).
                                    .on_scroll(move |e: &ScrollEvent| {
                                        let (y, h, total) = (f32::from(e.offset.y), f32::from(e.size.height), f32::from(e.content_size.height));
                                        if y + h > total - 600.0 && shown.get() < threads.get().len() {
                                            shown.set(shown.get() + 20);
                                        }
                                    })
                                    .child(
                                        view()
                                            .p_4()
                                            .gap_3()
                                            // Pestañas: un control segmentado compacto, con lo que espera en cada una; los hilos primero
                                            // (se abre en ellos).
                                            // Ajustes: su título (las áreas van en la barra de abajo).
                                            .child(dynamic(move || {
                                                if tab.get() != "settings" {
                                                    return view().hidden().into_element();
                                                }
                                                view()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_2()
                                                    .on_press(move || tab.set("threads"))
                                                    .child(text("‹").text_size(px(22.0)).text_color(c(tokens::FG)))
                                                    .child(text(crate::i18n::t("Settings")).text_size(px(16.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG)))
                                                    .into_element()
                                            }))
                                            .child(dynamic({
                                                let run_send = run_send.clone();
                                                move || {
                                                    if tab.get() != "jobs" {
                                                        return view().hidden().into_element();
                                                    }
                                                    let (recipes, flows) = run_catalog.get();
                                                    view().gap_3().child(ops::run_panel(run_form, envs.get(), recipes, flows, run_state.get(), run_send.clone())).child(ops::jobs(jobs.get(), job_open)).into_element()
                                                }
                                            }))
                                            .child(dynamic({
                                                let run_send = run_send.clone();
                                                move || {
                                                    if tab.get() != "envs" {
                                                        return view().hidden().into_element();
                                                    }
                                                    let ctx = ops::EnvsCtx {
                                                        overview: overview.get(),
                                                        site: site_overview.get(),
                                                        cloud_on: presence.get() == chat::Presence::Cloud,
                                                        running_jobs: jobs.get().iter().filter(|j| matches!(j.status.as_str(), "running" | "queued")).count(),
                                                        approvals: waiting_count.get(),
                                                        mac_on: presence.get() == chat::Presence::Mac,
                                                        state: run_state.get(),
                                                        asked: service_asked,
                                                        send: run_send.clone(),
                                                        open: open_projects,
                                                    };
                                                    ops::envs(envs.get(), ctx).into_element()
                                                }
                                            }))
                                            .child(dynamic(move || {
                                                if tab.get() != "approvals" {
                                                    return view().hidden().into_element();
                                                }
                                                if visible().is_empty() { card().child(muted("Nothing is waiting for your approval.")).into_element() } else { view().hidden().into_element() }
                                            }))
                                            .child(dynamic({
                                                let row = row.clone();
                                                move || {
                                                if tab.get() != "approvals" {
                                                    return view().hidden().into_element();
                                                }
                                                view().gap_3().child(each(
                                                    visible,
                                                    |a: &Approval| format!("{}|{:?}|{:?}|{}|{}", a.job_id, a.decision, a.phone_result, a.pending, a.status),
                                                    row.clone(),
                                                )).into_element()
                                            }}))
                                            .child(dynamic({
                                                let incident_row = incident_row.clone();
                                                move || {
                                                if tab.get() != "incidents" {
                                                    return view().hidden().into_element();
                                                }
                                                if incidents.get().is_empty() {
                                                    return card().child(status_tag(Health::Ok, "No incidents. Everything Condx watches is fine.")).into_element();
                                                }
                                                view().gap_3().child(each(
                                                    move || {
                                                        // También al abrir o cerrar algo, para que se rearmen las llaves.
                                                        let _ = (expanded.get(), menu_open.get(), answer_open.get(), deploy_pick.get());
                                                        incidents.get()
                                                    },
                                                    // Lo abierto (la tarjeta, su menú, la respuesta entera) va en la llave: la fila se rearma al cambiar.
                                                    move |i: &Incident| format!("{}|{}|{}|{:?}|{}|{}|{}|{}|{}|{}|{}|{}", i.id, i.status, i.count, i.result, i.waiting, i.agent, i.workflows.len(), expanded.get().contains(&i.id), menu_open.get().as_deref() == Some(i.id.as_str()), answer_open.get().contains(&i.id), deploy_pick.get().is_some_and(|d| d.0 == i.id), deploy_direct.get()),
                                                    incident_row.clone(),
                                                )).into_element()
                                            }}))
                                            .child(dynamic({
                                                move || {
                                                    if tab.get() != "threads" {
                                                        return view().hidden().into_element();
                                                    }
                                                    if thread_open.get().is_some() {
                                                        view().hidden().into_element()
                                                    } else {
                                                        {
                                                        // El aviso de versión nueva solo si hay (vacío sumaba otro espacio).
                                                        let latest = apk.get().filter(|(_, code, _)| *code > version_code(VERSION));
                                                        let mut v = view().gap_3();
                                                        if latest.is_some() {
                                                            v = v.child(update_banner(latest));
                                                        }
                                                        v.child(chat::list(threads.get(), thread_open, thread_text, loaded.get(), shown.get())).into_element()
                                                    }
                                                    }
                                                }
                                            }))
                                            // Este teléfono: abajo de aprobaciones e incidentes (en los hilos, el chat ocupa la pantalla).
                                            .child(dynamic({
                                                let unpair = unpair.clone();
                                                let run_send = run_send.clone();
                                                let slot_settings = slot_settings.clone();
                                                let slot_mcp = slot_mcp.clone();
                                                move || {
                                                if tab.get() != "settings" {
                                                    return view().hidden().into_element();
                                                }
                                                let scheme_now = scheme.get();
                                                let pick = move |v: &'static str| -> Box<dyn Fn()> {
                                                    Box::new(move || {
                                                        scheme.set(v.to_string());
                                                        theme::apply_scheme(v, true);
                                                    })
                                                };
                                                let lang = match agent_lang.get().as_str() {
                                                    "" => "Same as each message".to_string(),
                                                    "es" => "Español".to_string(),
                                                    "en" => "English".to_string(),
                                                    other => other.to_string(),
                                                };
                                                view()
                                                    .gap_3()
                                                    .child(section("Appearance"))
                                                    .child(theme::segmented(vec![
                                                        ("System", 0, scheme_now == "system", pick("system")),
                                                        ("Light", 0, scheme_now == "light", pick("light")),
                                                        ("Dark", 0, scheme_now == "dark", pick("dark")),
                                                    ]))
                                                    .child(section("Agents answer in"))
                                                    .child({
                                                        // Se manda firmado a la Mac (como un pedido); se ve enseguida.
                                                        let current = agent_lang.get();
                                                        let pick_lang = |code: &'static str| -> Box<dyn Fn()> {
                                                            let run_send = run_send.clone();
                                                            Box::new(move || {
                                                                agent_lang.set(code.to_string());
                                                                run_send(serde_json::json!({ "settings": { "language": code } }));
                                                            })
                                                        };
                                                        theme::segmented(vec![
                                                            ("Automatic", 0, current.is_empty(), pick_lang("")),
                                                            ("Español", 0, current == "es", pick_lang("es")),
                                                            ("English", 0, current == "en", pick_lang("en")),
                                                        ])
                                                    })
                                                    .child(muted(if lang == "Same as each message" { "Automatic: each agent answers in the language of the message." } else { "Every agent, on the Mac and in the cloud site, answers in this language." }))
                                                    .child(section("External agents"))
                                                    .child(mcp_section(slot_mcp.clone(), McpUi { conns: mcp_conns, url: mcp_url, loaded: mcp_loaded, form: mcp_form, name: mcp_name, level: mcp_level, code: mcp_code }))
                                                    .child(section("This phone")).child(
                                                card().rounded(px(10.0))
                                                    .child(kv("Key", mono(fingerprint.get(), 13.0)))
                                                    .child(kv("Stored in", text(secure::backend()).text_size(px(13.0))))
                                                    .child(dynamic({
                                                        let slot = slot_settings.clone();
                                                        move || {
                                                            let (pending, rejected, debug) = outbox.get();
                                                            let mut col = view().gap(px(4.0)).child(kv("Last send", text(last_send.get()).number_of_lines(2u32).text_size(px(12.0)))).child({
                                                                let p = LAST_PANIC.lock().map(|l| l.clone()).unwrap_or_default();
                                                                if p.is_empty() { view().into_element() } else { text(format!("panic: {p}")).number_of_lines(3u32).text_size(px(11.5)).text_color(c(tokens::DANGER)).into_element() }
                                                            }).child(kv("Sync", text(crate::i18n::tr(format!("{pending} waiting to send · {} rejected", rejected.len()), format!("{pending} por enviar · {} rechazados", rejected.len()))).text_size(px(13.0))));
                                                            for r in rejected.iter().take(4) {
                                                                col = col.child(text(r.clone()).number_of_lines(2u32).text_size(px(11.5)).text_color(c(tokens::DANGER)));
                                                            }
                                                            if pending > 0 {
                                                                col = col.child(text(debug).number_of_lines(3u32).text_size(px(11.0)).font_family(theme::MONO_FONT).text_color(c(tokens::SUBTLE)));
                                                            }
                                                            if !rejected.is_empty() {
                                                                let slot = slot.clone();
                                                                col = col.child(view().flex_row().child(theme::small_button("Discard rejected", Variant::Outline, move || {
                                                                    let slot = slot.clone();
                                                                    spawn(async move {
                                                                        let _ = with_phone(&slot, |p| p.discard_rejected()).await;
                                                                    });
                                                                })));
                                                            }
                                                            col.into_element()
                                                        }
                                                    }))
                                                    .child(dynamic(move || {
                                                        let ks = keys.get();
                                                        let content = if !can_keys.get() {
                                                            muted("Pair this phone again to receive provider keys.").into_element()
                                                        } else if ks.is_empty() {
                                                            muted("No provider keys yet.").into_element()
                                                        } else {
                                                            // Nombre en la fila del punto; el resto debajo, que sí se ajusta al ancho.
                                                            let mut col = view().gap_2();
                                                            for k in ks {
                                                                // Claude Code no tiene clave (usa la sesión de la Mac): solo el modelo.
                                                                let detail = if k.key_hint.is_empty() { k.model.clone() } else { format!("{} · key {}{}", k.model, k.key_hint, if k.opens { "" } else { " · can't open it here" }) };
                                                                col = col.child(view().gap_1().child(status_tag(if k.opens { Health::Ok } else { Health::Bad }, k.label)).child(muted(detail)));
                                                            }
                                                            col.into_element()
                                                        };
                                                        kv("Keys", content).into_element()
                                                    }))
                                                    .child(view().flex_row().justify_end().pt_1().child(theme::small_button("Unpair this phone", Variant::Outline, unpair.clone()))),
                                            
                                                ).into_element()
                                            }})),
                                    ))
                                    // Las áreas, abajo: lo que espera en cada una sobre su ícono.
                                    .child(dynamic(move || {
                                        let current = tab.get();
                                        let waiting_threads = threads.get().iter().filter(|t| t.waiting_answer || t.status == "waiting-approval").count();
                                        let running = jobs.get().iter().filter(|j| j.status == "running").count();
                                        let down = envs.get().iter().filter(|e| e.reachable == Some(false) && e.kind != "devbox").count();
                                        let open = incidents.get().iter().filter(|i| i.status == "open").count();
                                        theme::tab_bar(
                                            vec![
                                                ("threads", "Threads", theme::TAB_THREADS, waiting_threads),
                                                ("approvals", "Approvals", theme::TAB_APPROVALS, waiting_count.get()),
                                                ("jobs", "Jobs", theme::TAB_JOBS, running),
                                                ("envs", "Envs", theme::TAB_ENVS, down),
                                                ("incidents", "Incidents", theme::TAB_INCIDENTS, open),
                                            ],
                                            &current,
                                            move |k| {
                                                thread_open.set(None);
                                                tab.set(k);
                                                main_scroll.scroll_to(Point { x: Points(0.0), y: Points(0.0) }, false);
                                            },
                                        )
                                        .into_element()
                                    }))
                                    // Hilo nuevo: el botón flotante abajo a la derecha (en la lista de hilos).
                                    .child(dynamic(move || {
                                        if tab.get() != "threads" || thread_open.get().is_some() {
                                            return view().into_element();
                                        }
                                        view()
                                            .absolute()
                                            .right(px(20.0))
                                            // Encima de la barra de abajo (56).
                                            .bottom(px(76.0))
                                            .w(px(46.0))
                                            .h(px(46.0))
                                            .rounded(px(23.0))
                                            .items_center()
                                            .justify_center()
                                            .bg(c(tokens::ACCENT))
                                            .shadow_md()
                                            .active(|s| s.opacity(0.85))
                                            .on_press(move || {
                                                thread_text.set(String::new());
                                                thread_files.set(vec![]);
                                                thread_open.set(Some("_new".into()));
                                            })
                                            .child(theme::icon(theme::PLUS, tokens::ON_ACCENT, 21.0))
                                            .into_element()
                                    }))
                                    // Un hilo abierto: el chat a pantalla completa, encima.
                                    .child(dynamic({
                                        let send_thread = send_thread.clone();
                                        let composer = composer.clone();
                                        // Solo al abrir o cerrar un hilo se vuelve a montar; lo que llega
                                        // del hilo lo lee la pantalla (así el campo no pierde el foco).
                                        move || {
                                            let Some(id) = (tab.get() == "threads").then(|| thread_open.get()).flatten() else { return view().into_element() };
                                            let current: std::rc::Rc<dyn Fn() -> Option<Thread>> = {
                                                let id = id.clone();
                                                std::rc::Rc::new(move || {
                                                    if id == "_new" {
                                                        Some(new_thread.get().unwrap_or_else(|| Thread { id: "_new".into(), title: crate::i18n::t("New thread"), kind: "chat".into(), status: String::new(), updated_at: 0, items: vec![], participants: vec![], waiting_answer: false, project: None, env: None, result: None, waiting: false, sending: None, delivered: false, read: false, context: None }))
                                                    } else {
                                                        threads.get().into_iter().find(|t| t.id == id)
                                                    }
                                                })
                                            };
                                            view().absolute().top(px(0.0)).left(px(0.0)).right(px(0.0)).bottom(px(0.0)).child(chat::screen(id, current, thread_text, send_thread.clone(), composer.clone())).into_element()
                                        }
                                    }))
                                    .into_element(),
                            })),
                    ),
            ),
        )))
}

/// Esta app: su versión y su número (como el versionCode de la APK: MA·10000 + MI·100 + PA).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version_code(v: &str) -> i64 {
    let mut p = v.split('.').map(|x| x.parse::<i64>().unwrap_or(0));
    p.next().unwrap_or(0) * 10_000 + p.next().unwrap_or(0) * 100 + p.next().unwrap_or(0)
}

/// «Nueva versión de Condx · Instalar» si el daemon publicó una más nueva que esta: abre la descarga
/// (Android la instala sobre esta, con la misma firma).
fn update_banner(latest: Option<(String, i64, String)>) -> AnyElement {
    let Some((version, _, url)) = latest.filter(|(_, code, _)| *code > version_code(VERSION)) else { return view().into_element() };
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .py(px(10.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(c(tokens::ACCENT))
        .child(view().flex_1().gap(px(2.0)).child(text(crate::i18n::tr(format!("Condx {version} is ready"), format!("Condx {version} está lista"))).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))).child(text(crate::i18n::tr(format!("You have {VERSION}. Download it and Android installs it over this one."), format!("Tienes la {VERSION}. Descárgala y Android la instala sobre esta."))).text_size(px(12.0)).text_color(c(tokens::MUTED))))
        .child(
            view()
                .px_3()
                .h(px(32.0))
                .justify_center()
                .rounded(px(16.0))
                .bg(c(tokens::ACCENT))
                .active(|s| s.opacity(0.8))
                .on_press(move || {
                    if let Err(e) = scan::open_url(&url) {
                        log::error!("update: couldn't open the download: {e}");
                    }
                })
                .child(text(crate::i18n::t("Install")).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::ON_ACCENT))),
        )
        .into_element()
}

#[cfg(test)]
mod version_tests {
    #[test]
    fn el_numero_de_version_crece_con_la_version() {
        assert_eq!(super::version_code("0.1.0"), 100);
        assert_eq!(super::version_code("0.3.12"), 312);
        assert!(super::version_code("1.0.0") > super::version_code("0.99.99"));
    }
}

/// Lo que guarda la sección de agentes de fuera (Ajustes).
#[derive(Clone, Copy)]
struct McpUi {
    conns: Signal<Vec<(String, String, String, i64)>>,
    url: Signal<String>,
    loaded: Signal<bool>,
    form: Signal<bool>,
    name: Signal<String>,
    level: Signal<&'static str>,
    code: Signal<Option<Result<(String, String), String>>>,
}

/// Agentes de fuera (Claude Desktop en otra computadora, la web o el teléfono) conectados al MCP remoto
/// de Condx Cloud: quiénes están, desconectarlos y conectar otro con un código que firma este teléfono.
fn mcp_section(slot: Slot, ui: McpUi) -> AnyElement {
    let reload = {
        let slot = slot.clone();
        move || {
            let slot = slot.clone();
            spawn(async move {
                if let Some(Ok((url, list))) = with_phone(&slot, |p| p.mcp_connections()).await {
                    ui.url.set(url);
                    ui.conns.set(list);
                }
            });
        }
    };
    if !ui.loaded.get() {
        ui.loaded.set(true);
        reload();
    }
    dynamic(move || {
        let mut col = card().rounded(px(10.0)).gap(px(10.0));
        let list = ui.conns.get();
        if list.is_empty() {
            col = col.child(theme::muted("Connect Claude (or another agent) to work on your projects from anywhere, even with the Mac off."));
        }
        for (id, name, level, last) in list {
            let level_label = match level.as_str() {
                "orchestrate" => crate::i18n::tr("orchestrates".into(), "orquesta".into()),
                "reply" => crate::i18n::tr("reads and replies".into(), "lee y responde".into()),
                _ => crate::i18n::tr("reads".into(), "lee".into()),
            };
            let seen = if last > 0 { theme::ago(last) } else { crate::i18n::tr("not used yet".into(), "sin usar todavía".into()) };
            let (slot, reload) = (slot.clone(), reload.clone());
            col = col.child(
                view()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(view().flex_1().gap(px(2.0)).child(text(name).text_size(px(14.0)).text_color(c(tokens::FG))).child(text(format!("{level_label} · {seen}")).text_size(px(12.0)).text_color(c(tokens::MUTED))))
                    .child(theme::small_button("Disconnect", Variant::Outline, move || {
                        let (slot, id, reload) = (slot.clone(), id.clone(), reload.clone());
                        spawn(async move {
                            let _ = with_phone(&slot, move |p| p.mcp_disconnect(&id)).await;
                            reload();
                        });
                    })),
            );
        }
        match ui.code.get() {
            Some(Ok((code, url))) => {
                col = col
                    .child(view().h(px(1.0)).bg(c(tokens::CARD_BORDER)))
                    .child(theme::muted("In Claude: Settings → Connectors → Add custom connector, paste this URL and, when it asks to authorize, type the code (valid 10 minutes, once)."))
                    .child(text(url).font_family(theme::MONO_FONT).text_size(px(13.0)).text_color(c(tokens::FG)).selectable(true))
                    .child(text(code).font_family(theme::MONO_FONT).text_size(px(30.0)).font_weight(FontWeight::SEMIBOLD).text_color(c(tokens::ACCENT)).selectable(true))
                    .child(view().flex_row().child(theme::small_button("Done", Variant::Outline, {
                        let reload = reload.clone();
                        move || {
                            ui.code.set(None);
                            ui.form.set(false);
                            reload();
                        }
                    })));
            }
            Some(Err(e)) => col = col.child(text(e).number_of_lines(3u32).text_size(px(12.5)).text_color(c(tokens::DANGER))),
            None => {}
        }
        if ui.form.get() && !matches!(ui.code.get(), Some(Ok(_))) {
            let pick = |l: &'static str| -> Box<dyn Fn()> { Box::new(move || ui.level.set(l)) };
            let now = ui.level.get();
            let slot = slot.clone();
            col = col
                .child(theme::field("Name (e.g. Claude)", ui.name))
                .child(theme::segmented(vec![("Orchestrate", 0, now == "orchestrate", pick("orchestrate")), ("Reply", 0, now == "reply", pick("reply")), ("Read", 0, now == "read", pick("read"))]))
                .child(theme::muted(match now {
                    "orchestrate" => "Reads and writes in threads, calls Condx's agents, starts chats, tasks and flows. Never approves, touches production or reads keys.",
                    "reply" => "Reads threads and answers where you mention it.",
                    _ => "Only reads.",
                }))
                .child(view().flex_row().child(theme::small_button("Create code", Variant::Primary, move || {
                    let slot = slot.clone();
                    let (name, level) = (ui.name.get(), ui.level.get());
                    if name.trim().is_empty() {
                        return;
                    }
                    spawn(async move {
                        let r = with_phone(&slot, move |p| p.mcp_connect(&name, level)).await.unwrap_or_else(|| Err("not paired".into()));
                        ui.code.set(Some(r));
                    });
                })));
        } else if !ui.form.get() {
            col = col.child(view().flex_row().child(theme::small_button("Connect an agent", Variant::Outline, move || {
                ui.code.set(None);
                ui.form.set(true);
            })));
        }
        col.into_element()
    })
    .into_element()
}
