//! Condx en el teléfono (prueba de C4): las aprobaciones de Condx Cloud en vivo, y aprobar o
//! rechazar con una decisión firmada por la llave del teléfono. La red y el motor corren fuera del
//! hilo de la interfaz (`blocking`); la pantalla solo lee signals.
//!
//! Atajo SOLO de la prueba: usa el token del daemon desechable (variables de entorno al compilar,
//! nunca en el código). El emparejamiento real del teléfono (su propia llave, confirmada en la Mac)
//! es parte de C4.

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

const URL: Option<&str> = option_env!("CONDX_CLOUD_URL");
const DAEMON: Option<&str> = option_env!("CONDX_DAEMON_ID");
const TOKEN: Option<&str> = option_env!("CONDX_DAEMON_TOKEN");

type Slot = Arc<Mutex<Option<Phone>>>;

/// Corre `f` con el teléfono en un hilo del runtime y devuelve su resultado al hilo de la interfaz.
async fn with_phone<R: Send + 'static>(slot: &Slot, f: impl FnOnce(&mut Phone) -> R + Send + 'static) -> Option<R> {
    let slot = slot.clone();
    blocking(move || slot.lock().ok()?.as_mut().map(f)).await.ok().flatten()
}

#[component]
pub fn App() -> impl IntoElement {
    let approvals = signal(Vec::<Approval>::new());
    let status = signal(String::from("Connecting…"));
    let device = signal(String::new());
    let slot: Slot = Arc::new(Mutex::new(None));

    // Abrir y sincronizar cada 2 s.
    {
        let slot = slot.clone();
        spawn(async move {
            let (Some(url), Some(daemon), Some(token)) = (URL, DAEMON, TOKEN) else {
                status.set("Build with CONDX_CLOUD_URL, CONDX_DAEMON_ID and CONDX_DAEMON_TOKEN".into());
                return;
            };
            let opened = {
                let slot = slot.clone();
                blocking(move || match Phone::open(url, token, daemon) {
                    Ok(p) => {
                        let fp = p.fingerprint();
                        *slot.lock().unwrap() = Some(p);
                        Ok(fp)
                    }
                    Err(e) => Err(e),
                })
                .await
            };
            match opened {
                Ok(Ok(fp)) => device.set(format!("This phone · key {fp}…")),
                Ok(Err(e)) => {
                    status.set(format!("Couldn't open: {e}"));
                    return;
                }
                Err(_) => return,
            }
            loop {
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
                timer(Duration::from_secs(2)).await;
            }
        });
    }

    let decide = {
        let slot = slot.clone();
        move |job: String, approve: bool| {
            let slot = slot.clone();
            spawn(async move {
                let job2 = job.clone();
                if let Some(Err(e)) = with_phone(&slot, move |p| p.decide(&job2, approve)).await {
                    status.set(format!("Couldn't decide: {e}"));
                }
                if let Some(list) = with_phone(&slot, |p| p.approvals()).await {
                    approvals.set(list);
                }
            });
        }
    };

    // La clave lleva la decisión y el estado: una fila que cambia se vuelve a armar.
    let row = move |a: ReadSignal<Approval>, _ix: ReadSignal<usize>| {
        let decide = decide.clone();
        let decide2 = decide.clone();
        let item = a.get();
        let (job_a, job_r) = (item.job_id.clone(), item.job_id.clone());
        let waiting = item.status == "waiting-approval" && item.decision.is_none();
        view()
            .p_4()
            .gap_2()
            .rounded(px(14.0))
            .bg(move || theme().card)
            .child(text(item.title.clone()).text_size(px(17.0)))
            .child(text(format!("{} · {} · asked by {}", item.recipe, item.env, item.requested_by)).text_size(px(13.0)).text_color(move || theme().muted_foreground))
            .child(
                text(match (&item.decision, item.pending, item.status.as_str()) {
                    (Some(d), true, _) => format!("You chose {d} · sending…"),
                    (Some(d), false, _) => format!("You chose {d} · the Mac checks it"),
                    (None, _, "waiting-approval") => format!("Needs your approval ({})", item.level),
                    (None, _, s) => format!("Done: {s}"),
                })
                .text_size(px(13.0)),
            )
            .child(if waiting {
                Some(
                    view()
                        .flex_row()
                        .gap_3()
                        .child(
                            view()
                                .flex_1()
                                .py_3()
                                .items_center()
                                .rounded(px(10.0))
                                .bg(move || theme().primary)
                                .active(|s| s.opacity(0.6))
                                .on_press(move || decide(job_a.clone(), true))
                                .child(text("Approve").text_color(move || theme().primary_foreground)),
                        )
                        .child(
                            view()
                                .flex_1()
                                .py_3()
                                .items_center()
                                .rounded(px(10.0))
                                .bg(move || theme().secondary)
                                .active(|s| s.opacity(0.6))
                                .on_press(move || decide2(job_r.clone(), false))
                                .child(text("Reject")),
                        ),
                )
            } else {
                None
            })
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
                    .child(text(move || device.get()).text_size(px(12.0)).text_color(move || theme().muted_foreground))
                    .child(scroll().flex_1().child(view().gap_3().pb_4().child(each(move || approvals.get(), |a: &Approval| format!("{}|{:?}|{}|{}", a.job_id, a.decision, a.pending, a.status), row)))),
            ),
        )
}
