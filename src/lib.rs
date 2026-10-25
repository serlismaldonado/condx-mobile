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

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nsh_base::{safe_area_view, theme};
use nsh_core::ui::prelude::*;
use sync::{Approval, Phone};

#[native_module("condx-mobile/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
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

fn button(label: &'static str, primary: bool, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .flex_1()
        .py_3()
        .items_center()
        .rounded(px(10.0))
        .bg(move || if primary { theme().primary } else { theme().secondary })
        .active(|s| s.opacity(0.6))
        .on_press(on_press)
        .child(text(label).text_color(move || if primary { theme().primary_foreground } else { theme().foreground }))
}

fn field(placeholder: &'static str, value: Signal<String>) -> impl IntoElement {
    input()
        .value(move || value.get())
        .placeholder(placeholder)
        .auto_correct(false)
        .on_change(move |t: &str| value.set(t.to_string()))
        .px_3()
        .py_2()
        .rounded(px(8.0))
        .bg(move || theme().secondary)
}

#[component]
pub fn App() -> impl IntoElement {
    let screen = signal(Screen::Loading);
    let status = signal(String::new());
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
                                status.set(match pending {
                                    0 => format!("{waiting} waiting · in sync"),
                                    n => format!("{waiting} waiting · sending {n}…"),
                                });
                                approvals.set(list);
                            }
                            Some(Err(e)) => status.set(format!("Offline: {}", e.chars().take(80).collect::<String>())),
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

    let row = move |a: ReadSignal<Approval>, _ix: ReadSignal<usize>| {
        let (d1, d2) = (decide.clone(), decide.clone());
        let item = a.get();
        let (job_a, job_r) = (item.job_id.clone(), item.job_id.clone());
        let env = item.env.clone();
        let typed = item.level == "typed";
        // Producción (o aprobación reforzada): el teléfono solo propone; la Mac confirma con Touch ID.
        let prod = item.level == "reinforced" || item.env_kind == "cloud-prod";
        let waiting = item.status == "waiting-approval" && item.decision.is_none();
        let state = match (&item.decision, item.pending, item.phone_result.as_deref(), item.status.as_str()) {
            (Some(_), true, _, _) => "Sending your decision…".to_string(),
            (Some(_), false, Some("approved"), _) => "Approved from this phone".to_string(),
            (Some(_), false, Some("rejected the job"), _) => "Rejected from this phone".to_string(),
            (Some(_), false, Some("needs-mac"), _) => "Waiting for Touch ID on the Mac".to_string(),
            (Some(_), false, Some(r), _) => format!("The Mac didn't apply it: {}", r.trim_start_matches("rejected: ")),
            (Some(d), false, None, _) => format!("You chose {d} · the Mac checks it"),
            (None, _, _, "waiting-approval") => format!("Needs your approval ({})", if prod { "production: Touch ID on the Mac" } else { item.level.as_str() }),
            (None, _, _, s) => format!("Done: {s}"),
        };
        view()
            .p_4()
            .gap_2()
            .rounded(px(14.0))
            .border_1()
            .border_color(move || theme().border)
            .child(text(item.title.clone()).text_size(px(17.0)))
            .child(text(format!("{} · {} · asked by {}", item.recipe, item.env, item.requested_by)).text_size(px(13.0)).text_color(move || theme().muted_foreground))
            .child(text(state).text_size(px(13.0)))
            .child(if waiting && typed { Some(field("Type the environment's name to approve", confirm_text)) } else { None })
            .child(if waiting {
                Some(
                    view()
                        .flex_row()
                        .gap_3()
                        .child(button(if prod { "Propose" } else { "Approve" }, true, move || {
                            let confirm = typed.then(|| confirm_text.get().trim().to_string());
                            if typed && confirm.as_deref() != Some(env.as_str()) {
                                status.set(format!("Type “{env}” to approve"));
                                return;
                            }
                            confirm_text.set(String::new());
                            d1(job_a.clone(), true, confirm);
                        }))
                        .child(button("Reject", false, move || d2(job_r.clone(), false, None))),
                )
            } else {
                None
            })
    };

    let pair_view = move || {
        view()
            .gap_3()
            .child(text("Pair with your Mac").text_size(px(20.0)))
            .child(text("On the Mac: Condx → Settings → Condx Cloud → Pair a phone. Paste the QR's text, or type the code and the deployment URL.").text_size(px(14.0)).text_color(move || theme().muted_foreground))
            .child(field("Code (K7Q2-M9XD-4HWR) or the QR's text", pairing_text))
            .child(field("Deployment URL (only with a typed code)", pairing_url))
            .child(view().flex_row().child(button("Pair", true, do_pair)))
    };

    view()
        .flex_1()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(
            safe_area_view().flex_1().child(
                view()
                    .flex_1()
                    .px_4()
                    .pt_4()
                    .gap_3()
                    .child(text("Condx").text_size(px(28.0)))
                    .child(text(move || status.get()).text_size(px(14.0)).text_color(move || theme().muted_foreground))
                    .child(dynamic(move || match screen.get() {
                        Screen::Loading => view().child(text("…")).into_element(),
                        Screen::Pair => pair_view().into_element(),
                        Screen::Waiting(fp) => view()
                            .gap_3()
                            .child(text("Confirm on your Mac").text_size(px(20.0)))
                            .child(text("Check that the Mac shows this same key, then press Confirm there.").text_size(px(14.0)).text_color(move || theme().muted_foreground))
                            .child(text(fp).text_size(px(24.0)))
                            .into_element(),
                        Screen::Ready => view()
                            .flex_1()
                            .gap_3()
                            .child(text(move || format!("This phone · key {} · {}", fingerprint.get(), secure::backend())).text_size(px(12.0)).text_color(move || theme().muted_foreground))
                            .child(scroll().flex_1().child(view().gap_3().pb_4().child(each(
                                move || approvals.get(),
                                |a: &Approval| format!("{}|{:?}|{:?}|{}|{}", a.job_id, a.decision, a.phone_result, a.pending, a.status),
                                row.clone(),
                            ))))
                            .child(view().flex_row().pb_4().child(button("Unpair this phone", false, unpair.clone())))
                            .into_element(),
                    })),
            ),
        )
}
