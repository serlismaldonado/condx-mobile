//! Condx en el teléfono: vincularlo con tu Mac y aprobar (o rechazar) lo que espera tu aprobación.
//!
//! - **Vincular**: el código que muestra la Mac (con su QR). El teléfono genera su llave Ed25519 y su
//!   token, reclama el código y espera a que confirmes en la Mac (comparando la huella).
//! - **Identidad**: Keychain en iOS, Keystore en Android (`secure.rs`).
//! - **Aprobar**: la decisión sale firmada; la Mac la verifica con la llave que confirmó. En
//!   producción el teléfono solo propone: la Mac confirma con Touch ID.
//!
//! La red y el almacenamiento corren fuera del hilo de la interfaz (`blocking`).

pub mod scan;
pub mod secure;
pub mod sync;
pub mod theme;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_base::safe_area_view;
use nsh_core::ui::prelude::*;
use sync::{Approval, Incident, Phone, ProviderKey};
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

async fn with_phone<R: Send + 'static>(slot: &Slot, f: impl FnOnce(&mut Phone) -> R + Send + 'static) -> Option<R> {
    let slot = slot.clone();
    blocking(move || slot.lock().ok()?.as_mut().map(f)).await.ok().flatten()
}

#[component]
pub fn App() -> impl IntoElement {
    let screen = signal(Screen::Loading);
    // Mensajes para la persona (errores, avisos) y el estado de la sincronización (barra superior).
    let status = signal(String::new());
    let sync_line = signal(String::new());
    let waiting_count = signal(0usize);
    // Avanza en cada vuelta del ciclo: los «hace N min» se vuelven a calcular.
    let tick = signal(0u64);
    let approvals = signal(Vec::<Approval>::new());
    // Incidentes (A0b): la lista, la pestaña elegida, las tarjetas abiertas y el mensaje al agente.
    let incidents = signal(Vec::<Incident>::new());
    let tab = signal("approvals");
    let expanded = signal(Vec::<String>::new());
    let agent_text = signal(String::new());
    // Claves de proveedores cifradas para este teléfono (A6): cuáles llegaron y si se abren aquí.
    let keys = signal(Vec::<ProviderKey>::new());
    let can_keys = signal(true);
    let fingerprint = signal(String::new());
    let slot: Slot = Arc::new(Mutex::new(None));
    // Vincular: el texto del QR (o solo el código) y la URL del deployment si hace falta.
    let pairing_text = signal(String::new());
    let pairing_url = signal(String::new());

    // Arranque y ciclo: identidad → (pendiente: esperar) → sincronizar cada 2 s.
    {
        let slot = slot.clone();
        spawn(async move {
            loop {
                let id = blocking(secure::load).await.ok().and_then(|r| r.ok()).flatten();
                let Some(id) = id else {
                    if screen.get() != Screen::Pair {
                        screen.set(Screen::Pair);
                    }
                    timer(Duration::from_secs(1)).await;
                    continue;
                };
                fingerprint.set(sync::fingerprint_of(&id));
                let state = {
                    let id = id.clone();
                    blocking(move || sync::device_status(&id)).await.ok()
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
                                status.set(format!("Couldn't open: {e}"));
                            }
                        }
                        screen.set(Screen::Ready);
                        match with_phone(&slot, |p| p.sync().map(|list| (list, p.pending_changes(), p.incidents(), p.keys(), p.can_receive_keys()))).await {
                            Some(Ok((list, pending, incs, ks, can))) => {
                                incidents.set(incs);
                                if keys.get() != ks {
                                    keys.set(ks);
                                }
                                can_keys.set(can);
                                let waiting = list.iter().filter(|a| a.status == "waiting-approval").count();
                                waiting_count.set(waiting);
                                if status.get().starts_with("Offline") {
                                    status.set(String::new());
                                }
                                sync_line.set(match pending {
                                    0 => "in sync".to_string(),
                                    n => format!("sending {n}…"),
                                });
                                approvals.set(list);
                            }
                            Some(Err(e)) => {
                                sync_line.set("offline".into());
                                status.set(format!("Offline: {}", e.chars().take(80).collect::<String>()));
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
                timer(Duration::from_secs(2)).await;
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
                    status.set(format!("Couldn't decide: {e}"));
                }
                if let Some(list) = with_phone(&slot, |p| p.approvals()).await {
                    approvals.set(list);
                }
            });
        }
    };

    // Un pedido sobre un incidente: sale firmado en el próximo sync; el daemon decide.
    let ask = {
        let slot = slot.clone();
        move |id: String, request: &'static str, arg: String| {
            let slot = slot.clone();
            spawn(async move {
                if let Some(Err(e)) = with_phone(&slot, move |p| p.request(&id, request, &arg)).await {
                    status.set(format!("Couldn't send: {e}"));
                }
                if let Some(list) = with_phone(&slot, |p| p.incidents()).await {
                    incidents.set(list);
                }
            });
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
    let visible = move || approvals.get().into_iter().filter(|a| a.status == "waiting-approval" || a.decision.is_some()).collect::<Vec<_>>();

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
            (Some(_), false, Some("rejected the job"), _) => (Health::Bad, "Rejected from this phone".to_string()),
            (Some(_), false, Some("needs-mac"), _) => (Health::Warn, "Waiting for Touch ID on the Mac".to_string()),
            (Some(_), false, Some(r), _) => (Health::Bad, format!("The Mac didn't apply it: {}", r.trim_start_matches("rejected: "))),
            (Some(d), false, None, _) => (Health::Unknown, format!("You chose {d} · the Mac checks it")),
            (None, _, _, "waiting-approval") => (Health::Warn, format!("Needs your approval · {}", if prod { "production: Touch ID on the Mac" } else { item.level.as_str() })),
            (None, _, _, s) => (Health::Unknown, format!("Done: {s}")),
        };
        let (requested_by, recipe, created_at) = (item.requested_by.clone(), item.recipe.clone(), item.created_at);
        let summary = move || {
            tick.get();
            format!("requested by {requested_by}{} · recipe {recipe}", if created_at > 0 { format!(" {}", theme::ago(created_at)) } else { String::new() })
        };
        card()
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(view().flex_1().child(text(format!("{} · {}", item.title, item.env)).text_size(px(14.0)).text_color(c(tokens::FG))))
                    .child(kind_tag(&item.env_kind)),
            )
            .child(text(summary).text_size(px(12.5)).text_color(c(tokens::MUTED)))
            .child(status_tag(health, state))
            .child(match (waiting, typed && !prod) {
                // «typed»: en la Mac se escribe el nombre del entorno; aquí se desliza hasta el final
                // (igual de deliberado) y la decisión lleva el nombre que el daemon compara.
                (true, true) => Some(
                    view()
                        .gap_2()
                        .pt_1()
                        .child(theme::slide_to_confirm("Slide to approve", move || d1(job_a.clone(), true, Some(env.clone()))))
                        .child(view().flex_row().child(theme::button("Reject", Variant::Danger, move || d2(job_r.clone(), false, None))))
                        .into_element(),
                ),
                (true, false) => Some(
                    view()
                        .flex_row()
                        .gap_2()
                        .pt_1()
                        .child(theme::button(if prod { "Propose" } else { "Approve" }, Variant::Primary, move || d1(job_a.clone(), true, None)))
                        .child(theme::button("Reject", Variant::Danger, move || d2(job_r.clone(), false, None)))
                        .into_element(),
                ),
                (false, _) => None,
            })
    };

    // Una tarjeta de incidente, con lo que hizo el agente y lo que se puede pedir.
    let incident_row = move |i: ReadSignal<Incident>, _ix: ReadSignal<usize>| {
        let inc = i.get();
        let agent = inc.agent.clone();
        let agent_status = agent["status"].as_str().unwrap_or("").to_string();
        let running = agent_status == "running";
        let open = inc.status != "resolved";
        let fix = agent["fix"].clone();
        let fix_ready = fix["changes"] == true && fix["discarded"] != true && fix["commit"].is_string();
        let is_expanded = expanded.get().contains(&inc.id);
        let mut k = card().child(
            view()
                .flex_row()
                .gap_2()
                .items_center()
                .child(view().flex_1().child(text(inc.title.clone()).text_size(px(14.0)).text_color(c(tokens::FG))))
                .child(kind_tag(&inc.source)),
        );
        let when = theme::ago(inc.last_seen);
        let meta = [(!inc.env.is_empty()).then(|| format!("on {}", inc.env)), (inc.count > 1).then(|| format!("{} times", inc.count)), Some(when), (inc.status == "ignored").then(|| "ignored".to_string()), (inc.status == "resolved").then(|| "resolved".to_string())];
        k = k.child(muted(meta.into_iter().flatten().collect::<Vec<_>>().join(" · ")));
        if !inc.error.is_empty() {
            let first: String = inc.error.lines().next().unwrap_or("").chars().take(140).collect();
            k = k.child(text(first).font_family(theme::MONO_FONT).text_size(px(11.5)).text_color(c(tokens::PROD)));
        }
        // El agente: estado, plantilla y su respuesta (la causa; toda al abrirla).
        if agent.is_object() {
            let (h, label) = match (agent_status.as_str(), agent["mode"].as_str()) {
                ("running", Some("fix")) => (Health::Warn, "An agent is preparing a fix".to_string()),
                ("running", _) => (Health::Warn, "An agent is diagnosing it".to_string()),
                ("done", Some("fix")) => (Health::Ok, format!("Fix prepared by {}", agent["model"].as_str().unwrap_or("the agent"))),
                ("done", _) => (Health::Ok, format!("Diagnosis by {}", agent["model"].as_str().unwrap_or("the agent"))),
                ("stopped", _) => (Health::Unknown, "The agent stopped (limit or budget)".to_string()),
                ("aborted", _) => (Health::Unknown, "The agent was stopped".to_string()),
                _ => (Health::Bad, "The agent failed".to_string()),
            };
            k = k.child(status_tag(h, label));
            if let Some(answer) = agent["answer"].as_str().filter(|a| !a.trim().is_empty()) {
                let shown: String = if is_expanded {
                    answer.to_string()
                } else {
                    answer.split("## ").find(|s| s.starts_with("Cause")).map(|s| s.trim_start_matches("Cause").trim().to_string()).unwrap_or_else(|| answer.chars().take(240).collect())
                };
                let mut col = view().gap_1();
                for line in shown.lines().filter(|l| !l.trim().is_empty()) {
                    col = col.child(match line.strip_prefix("## ") {
                        Some(h) => section(h).into_element(),
                        None => text(line.to_string()).text_size(px(13.0)).text_color(c(tokens::FG)).into_element(),
                    });
                }
                let id = inc.id.clone();
                k = k.child(col).child(
                    view()
                        .flex_row()
                        .child(text(if is_expanded { "Show less" } else { "Show the whole answer" }).text_size(px(12.5)).text_color(c(tokens::MUTED)))
                        .on_press(move || {
                            let mut e = expanded.get();
                            if let Some(ix) = e.iter().position(|x| x == &id) { e.remove(ix); } else { e.push(id.clone()); }
                            expanded.set(e);
                        }),
                );
            }
            if fix_ready {
                k = k.child(status_tag(Health::Ok, format!("Fix ready on {} · {} files", fix["branch"].as_str().unwrap_or("?"), fix["files"].as_array().map(Vec::len).unwrap_or(0))));
                if let Some(stat) = fix["stat"].as_str() {
                    k = k.child(text(stat.to_string()).font_family(theme::MONO_FONT).text_size(px(11.0)).text_color(c(tokens::MUTED)));
                }
            }
        }
        // Lo que hizo el daemon con el último pedido de este teléfono.
        if inc.waiting {
            k = k.child(status_tag(Health::Warn, "Sending your request…"));
        } else if let Some(r) = &inc.result {
            k = k.child(status_tag(if r.starts_with("rejected") { Health::Bad } else { Health::Ok }, r.clone()));
        }
        if open && !inc.waiting && !running {
            let (a1, a2, a3, a4) = (ask.clone(), ask.clone(), ask.clone(), ask.clone());
            let (id1, id2, id3, id4) = (inc.id.clone(), inc.id.clone(), inc.id.clone(), inc.id.clone());
            k = k
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .pt_1()
                        .child(theme::button(if agent.is_object() { "Diagnose again" } else { "Diagnose" }, Variant::Primary, move || a1(id1.clone(), "diagnose", String::new())))
                        .child(theme::button("Prepare a fix", Variant::Outline, move || a2(id2.clone(), "fix", String::new()))),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(theme::button("Ignore", Variant::Outline, move || a3(id3.clone(), "ignore", String::new())))
                        .child(theme::button("Resolve", Variant::Outline, move || a4(id4.clone(), "resolve", String::new()))),
                );
            // Desplegar el arreglo: deslizar (el flujo pide sus propias aprobaciones).
            if fix_ready {
                for (wid, name) in inc.workflows.iter().take(3) {
                    let (a, id, wid) = (ask.clone(), inc.id.clone(), wid.clone());
                    k = k.child(theme::slide_to_confirm(format!("Slide to run “{name}”"), move || a(id.clone(), "deploy", wid.clone())));
                }
                if inc.workflows.is_empty() {
                    k = k.child(muted("Turn on a workflow on the Mac to deploy fixes from here."));
                }
                let (a, id) = (ask.clone(), inc.id.clone());
                k = k.child(view().flex_row().child(theme::button("Discard the fix", Variant::Danger, move || a(id.clone(), "discard", String::new()))));
            }
        }
        // Escribirle al agente (sigue la conversación o la redirige si está trabajando).
        if agent.is_object() && open && !inc.waiting {
            let (a, id) = (ask.clone(), inc.id.clone());
            k = k.child(
                view()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(view().flex_1().child(field("Write to the agent…", agent_text)))
                    .child(view().w(px(84.0)).flex_row().child(theme::button("Send", Variant::Outline, move || {
                        let text = agent_text.get().trim().to_string();
                        if text.is_empty() {
                            return;
                        }
                        agent_text.set(String::new());
                        a(id.clone(), "message", text);
                    }))),
            );
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
            .child(view().w(px(10.0)).h(px(10.0)).bg(c(tokens::ACCENT)))
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
                        .child(text(format!("{n} to approve")).text_size(px(12.0)))
                        .into_element()
                } else {
                    view().into_element()
                }
            }))
            .child(dynamic(move || {
                let line = sync_line.get();
                let health = match line.as_str() {
                    "in sync" => Health::Ok,
                    "offline" => Health::Bad,
                    "" => Health::Unknown,
                    _ => Health::Warn,
                };
                if screen.get() == Screen::Ready { status_tag(health, line).into_element() } else { view().into_element() }
            }))
    };

    view()
        .flex_1()
        .bg(c(tokens::GROUND))
        .text_color(c(tokens::FG))
        .font_family(theme::UI_FONT)
        .text_size(px(14.0))
        .child(
            safe_area_view().flex_1().child(
                view()
                    .flex_1()
                    .child(top_bar())
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
                                Screen::Ready => scroll()
                                    .flex_1()
                                    .child(
                                        view()
                                            .p_4()
                                            .gap_3()
                                            // Pestañas: aprobaciones e incidentes.
                                            .child(dynamic(move || {
                                                let open = incidents.get().iter().filter(|i| i.status == "open").count();
                                                let incidents_label = if open > 0 { format!("Incidents · {open}") } else { "Incidents".to_string() };
                                                view()
                                                    .flex_row()
                                                    .gap_2()
                                                    .child(theme::button("Approvals", if tab.get() == "approvals" { Variant::Primary } else { Variant::Outline }, move || tab.set("approvals")))
                                                    .child(theme::button(incidents_label, if tab.get() == "incidents" { Variant::Primary } else { Variant::Outline }, move || tab.set("incidents")))
                                                    .into_element()
                                            }))
                                            .child(dynamic(move || {
                                                if tab.get() != "approvals" {
                                                    return view().into_element();
                                                }
                                                if visible().is_empty() { card().child(muted("Nothing is waiting for your approval.")).into_element() } else { view().into_element() }
                                            }))
                                            .child(dynamic({
                                                let row = row.clone();
                                                move || {
                                                if tab.get() != "approvals" {
                                                    return view().into_element();
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
                                                    return view().into_element();
                                                }
                                                if incidents.get().is_empty() {
                                                    return card().child(status_tag(Health::Ok, "No incidents. Everything Condx watches is fine.")).into_element();
                                                }
                                                view().gap_3().child(each(
                                                    move || incidents.get(),
                                                    |i: &Incident| format!("{}|{}|{}|{:?}|{}|{}|{}", i.id, i.status, i.count, i.result, i.waiting, i.agent, i.workflows.len()),
                                                    incident_row.clone(),
                                                )).into_element()
                                            }}))
                                            .child(view().pt_2().child(section("This phone")))
                                            .child(
                                                card()
                                                    .child(kv("Key", mono(fingerprint.get(), 13.0)))
                                                    .child(kv("Stored in", text(secure::backend()).text_size(px(13.0))))
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
                                                                let detail = format!("{} · key {}{}", k.model, k.key_hint, if k.opens { "" } else { " · can't open it here" });
                                                                col = col.child(view().gap_1().child(status_tag(if k.opens { Health::Ok } else { Health::Bad }, k.label)).child(muted(detail)));
                                                            }
                                                            col.into_element()
                                                        };
                                                        kv("Keys", content).into_element()
                                                    }))
                                                    .child(view().flex_row().pt_1().child(theme::button("Unpair this phone", Variant::Outline, unpair.clone()))),
                                            ),
                                    )
                                    .into_element(),
                            })),
                    ),
            ),
        )
}
