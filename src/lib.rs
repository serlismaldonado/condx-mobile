//! Condx en el teléfono: vincularlo con tu Mac y aprobar (o rechazar) lo que espera tu aprobación.
//!
//! - **Vincular**: el código que muestra la Mac (con su QR). El teléfono genera su llave Ed25519 y su
//!   token, reclama el código y espera a que confirmes en la Mac (comparando la huella).
//! - **Identidad**: Keychain en iOS, Keystore en Android (`secure.rs`).
//! - **Aprobar**: la decisión sale firmada; la Mac la verifica con la llave que confirmó. En
//!   producción el teléfono solo propone: la Mac confirma con Touch ID.
//!
//! La red y el almacenamiento corren fuera del hilo de la interfaz (`blocking`).

pub mod secure;
pub mod sync;
pub mod theme;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_base::safe_area_view;
use nsh_core::ui::prelude::*;
use sync::{Approval, Phone};
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
    let approvals = signal(Vec::<Approval>::new());
    let fingerprint = signal(String::new());
    let slot: Slot = Arc::new(Mutex::new(None));
    // Vincular: el texto del QR (o solo el código) y la URL del deployment si hace falta.
    let pairing_text = signal(String::new());
    let pairing_url = signal(String::new());
    // Confirmación escrita (aprobaciones «typed»), por trabajo.
    let confirm_text = signal(String::new());

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
                        match with_phone(&slot, |p| p.sync().map(|list| (list, p.pending_changes()))).await {
                            Some(Ok((list, pending))) => {
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
        let summary = format!(
            "requested by {}{} · recipe {}",
            item.requested_by,
            if item.created_at > 0 { format!(" {}", theme::ago(item.created_at)) } else { String::new() },
            item.recipe
        );
        card()
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(view().flex_1().child(text(format!("{} · {}", item.title, item.env)).text_size(px(14.0)).text_color(c(tokens::FG))))
                    .child(kind_tag(&item.env_kind)),
            )
            .child(muted(summary))
            .child(status_tag(health, state))
            .child(if waiting && typed { Some(kv("Confirm", field("Type the environment's name", confirm_text))) } else { None })
            .child(if waiting {
                Some(
                    view()
                        .flex_row()
                        .gap_2()
                        .pt_1()
                        .child(theme::button(if prod { "Propose" } else { "Approve" }, Variant::Primary, move || {
                            let confirm = typed.then(|| confirm_text.get().trim().to_string());
                            if typed && confirm.as_deref() != Some(env.as_str()) {
                                status.set(format!("Type “{env}” to approve"));
                                return;
                            }
                            confirm_text.set(String::new());
                            d1(job_a.clone(), true, confirm);
                        }))
                        .child(theme::button("Reject", Variant::Danger, move || d2(job_r.clone(), false, None))),
                )
            } else {
                None
            })
    };

    let pair_view = move || {
        view()
            .gap_3()
            .child(section("Pair with your Mac"))
            .child(
                card()
                    .gap_3()
                    .child(muted("On the Mac: Condx → Settings → Condx Cloud → Pair a phone. Paste the QR's text, or type the code and the deployment URL."))
                    .child(field("Code (K7Q2-M9XD-4HWR) or the QR's text", pairing_text).auto_capitalize(AutoCapitalize::None))
                    .child(field("Deployment URL (only with a typed code)", pairing_url).auto_capitalize(AutoCapitalize::None).keyboard_type(KeyboardType::Url))
                    .child(view().flex_row().child(theme::button("Pair", Variant::Primary, do_pair))),
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
                                            .child(section("Approvals"))
                                            .child(dynamic(move || {
                                                if visible().is_empty() { card().child(muted("Nothing is waiting for your approval.")).into_element() } else { view().into_element() }
                                            }))
                                            .child(view().gap_3().child(each(
                                                visible,
                                                |a: &Approval| format!("{}|{:?}|{:?}|{}|{}", a.job_id, a.decision, a.phone_result, a.pending, a.status),
                                                row.clone(),
                                            )))
                                            .child(view().pt_2().child(section("This phone")))
                                            .child(
                                                card()
                                                    .child(kv("Key", mono(fingerprint.get(), 13.0)))
                                                    .child(kv("Stored in", text(secure::backend()).text_size(px(13.0))))
                                                    .child(view().flex_row().pt_1().child(theme::button("Unpair this phone", Variant::Outline, unpair.clone()))),
                                            ),
                                    )
                                    .into_element(),
                            })),
                    ),
            ),
        )
}
