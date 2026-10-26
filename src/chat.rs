//! Los hilos (F6) como un chat: la lista de conversaciones y, abierta una, sus mensajes a pantalla
//! completa con el campo para escribir fijo abajo. Los agentes hablan en burbujas con su inicial;
//! lo tuyo va a la derecha en el amarillo de la marca; el error, la pregunta del flujo y lo que
//! espera aprobación son bloques con una franja de color; lo demás, una línea centrada y apagada.

use nsh_core::ui::prelude::*;
use serde_json::Value;

use crate::sync::Thread;
use crate::theme::{self, c, muted, now, tokens, Health};

pub fn thread_health(status: &str) -> Health {
    match status {
        "running" | "waiting-approval" | "waiting-answer" => Health::Warn,
        "succeeded" | "done" => Health::Ok,
        "failed" | "stopped" => Health::Bad,
        _ => Health::Unknown,
    }
}

pub fn thread_state(t: &Thread) -> String {
    match t.status.as_str() {
        _ if t.waiting_answer => "Waiting for your answer".into(),
        "running" => "Working…".into(),
        "waiting-approval" => "Waiting for your approval".into(),
        "waiting-answer" => "Waiting for your answer".into(),
        "succeeded" => "Done".into(),
        "done" => "Waiting for you".into(),
        "cancelled" => "Cancelled".into(),
        "failed" => "Failed".into(),
        "stopped" => "Stopped".into(),
        other => other.to_string(),
    }
}

/// Un color por nombre, siempre el mismo para el mismo agente.
fn hue(name: &str) -> (u32, u32) {
    // Los agentes de fábrica, siempre con el mismo; lo demás (los flujos), por su nombre.
    match name.to_lowercase().as_str() {
        "triage" => return tokens::INFO,
        "risk" => return tokens::PROD,
        "fixer" => return tokens::SUCCESS,
        "reviewer" => return tokens::VIOLET,
        "explainer" | "agent" => return tokens::ACCENT,
        _ => {}
    }
    let name = name.split(" · ").next().unwrap_or(name);
    const HUES: [(u32, u32); 5] = [tokens::ACCENT, tokens::INFO, tokens::SUCCESS, tokens::VIOLET, tokens::PROD];
    HUES[name.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize)) % HUES.len()]
}

fn avatar(name: &str, size: f32) -> impl IntoElement {
    let initial = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "·".into());
    view()
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
        .rounded(px(size / 2.0))
        .items_center()
        .justify_center()
        .bg(c(hue(name)))
        .child(text(initial).text_size(px(size * 0.45)).font_weight(FontWeight::SEMIBOLD).text_color(c(tokens::ON_ACCENT)))
}

/// «hace 3 min» corto, para la hora de un mensaje.
fn short_ago(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    theme::ago(ms).trim_end_matches(" ago").to_string()
}

/// El texto de un agente: la prosa arriba y los campos de su resultado (`clave: valor`) como filas.
fn agent_body(body: &str) -> View {
    let mut prose = Vec::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in body.lines() {
        match line.split_once(": ") {
            Some((k, v)) if !k.is_empty() && k.len() <= 24 && k.chars().all(|c| c.is_ascii_lowercase() || c == '_') => {
                let v = v.trim();
                if !v.is_empty() {
                    fields.push((k.to_string(), if k == "commit" { v.chars().take(10).collect() } else { v.to_string() }));
                }
            }
            _ if !line.trim().is_empty() => prose.push(line.trim().to_string()),
            _ => {}
        }
    }
    // El resumen primero; lo demás en orden fijo (llegan en cualquier orden).
    fields.sort_by(|a, b| (a.0 != "summary", &a.0).cmp(&(b.0 != "summary", &b.0)));
    let mut col = view().gap_2();
    if !prose.is_empty() {
        col = col.child(text(prose.join("\n")).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::FG)));
    }
    if !fields.is_empty() {
        let mut grid = view().gap(px(8.0));
        for (k, v) in fields {
            let tone = match (k.as_str(), v.as_str()) {
                (_, "critical" | "high") if k.contains("risk") || k == "severity" => Some(tokens::DANGER),
                (_, "medium") if k.contains("risk") || k == "severity" => Some(tokens::WARNING),
                (_, "low") if k.contains("risk") || k == "severity" => Some(tokens::SUCCESS),
                (_, "true") => Some(tokens::SUCCESS),
                (_, "false") => Some(tokens::MUTED),
                _ => None,
            };
            let v_len = v.chars().count();
            let value = text(v).text_size(px(13.5)).line_height(px(19.0)).text_color(c(tone.unwrap_or(tokens::FG)));
            // Corto: en la misma fila que su nombre; largo, debajo.
            let label = text(k.replace('_', " ")).text_size(px(11.5)).line_height(px(19.0)).text_color(c(tokens::SUBTLE));
            let value = if tone.is_some() { value.font_weight(FontWeight::MEDIUM) } else { value };
            grid = grid.child(if v_len <= 28 {
                view().flex_row().gap_3().child(view().w(px(92.0)).flex_shrink_0().child(label)).child(view().flex_1().child(value))
            } else {
                view().gap(px(1.0)).child(label).child(value)
            }            );
        }
        col = col.child(view().pt_1().border_t_1().border_color(c(tokens::CARD_BORDER)).child(view().pt_2().child(grid)));
    }
    col
}

/// Un bloque con franja de color a la izquierda: el error, la pregunta, la aprobación.
fn banner(tone: (u32, u32), label: &str, body: View) -> View {
    view()
        .flex_row()
        .rounded(px(8.0))
        .overflow_hidden()
        .bg(c(tokens::HOVER))
        .child(view().w(px(3.0)).bg(c(tone)))
        .child(view().flex_1().gap_1().px_3().py_2().child(text(label.to_uppercase()).text_size(px(10.5)).letter_spacing(px(0.6)).font_weight(FontWeight::MEDIUM).text_color(c(tone))).child(body))
}

fn message(it: &Value, last_question: bool, quick: &dyn Fn(&'static str) -> View) -> AnyElement {
    let body = it["text"].as_str().unwrap_or_default().trim().to_string();
    let at = it["at"].as_i64().unwrap_or(0);
    match it["kind"].as_str().unwrap_or("system") {
        "user" => {
            let to = it["to"].as_str().filter(|t| !t.is_empty()).map(|t| format!("to {t} · ")).unwrap_or_default();
            view()
                .items_end()
                .pl(px(56.0))
                .gap_1()
                .child(
                    view()
                        .px_3()
                        .py_2()
                        .rounded(px(14.0))
                        .rounded_br(px(4.0))
                        .bg(c(tokens::ACCENT))
                        .child(text(body).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::ON_ACCENT))),
                )
                .child(text(format!("{to}{}", short_ago(at))).text_size(px(11.0)).text_color(c(tokens::SUBTLE)))
                .into_element()
        }
        "agent" | "reply" => {
            let name = it["author"].as_str().unwrap_or("Agent").to_string();
            view()
                .flex_row()
                .gap_2()
                .pr(px(24.0))
                .child(avatar(&name, 30.0))
                .child(
                    view()
                        .flex_1()
                        .gap_1()
                        .child(
                            view()
                                .flex_row()
                                .gap_2()
                                .items_center()
                                .child(text(name).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG)))
                                .child(text(short_ago(at)).text_size(px(11.0)).text_color(c(tokens::SUBTLE))),
                        )
                        .child(view().px_3().py(px(10.0)).rounded(px(14.0)).rounded_tl(px(4.0)).bg(c(tokens::HOVER)).child(agent_body(&body))),
                )
                .into_element()
        }
        "event" => {
            let body = body.trim_end_matches(" ·").to_string();
            match it["source"].as_str() {
                Some("task") => banner(tokens::INFO, "Task", view().child(text(body).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::FG)))),
                Some("user-report") => banner(tokens::WARNING, "User report", view().child(text(body).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::FG)))),
                source => banner(
                    tokens::DANGER,
                    match source {
                        Some("convex-error") => "Convex error",
                        Some("frontend-error") => "Frontend error",
                        Some("incident") => "Incident",
                        _ => "Error",
                    },
                    view().child(text(body).font_family(theme::MONO_FONT).text_size(px(12.5)).line_height(px(18.0)).text_color(c(tokens::FG))),
                ),
            }
            .into_element()
        }
        "question" => {
            // Respondida si el hilo ya no espera esta (la nube no manda «answered»).
            let answered = it["answered"] == true || !last_question;
            let mut b = view().gap_2().child(text(body).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::FG)));
            if last_question && !answered {
                b = b.child(view().flex_row().gap_2().pt_1().child(quick("Yes")).child(quick("No")));
            }
            banner(if answered { tokens::SUBTLE } else { tokens::WARNING }, if answered { "The flow asked" } else { "The flow asks you" }, b).into_element()
        }
        "approval" => banner(
            tokens::WARNING,
            "Needs your approval",
            view().gap_1().child(text(body).text_size(px(14.0)).text_color(c(tokens::FG))).child(muted("Approve or reject it in the Approvals tab.")),
        )
        .into_element(),
        _ => {
            let running = it["status"] == "running";
            let failed = it["status"] == "failed";
            view()
                .flex_row()
                .justify_center()
                .items_center()
                .gap_2()
                .px_4()
                .child(if running || failed { theme::dot(if failed { Health::Bad } else { Health::Warn }).into_element() } else { view().into_element() })
                .child(text(body).text_size(px(12.0)).text_center().text_color(c(if failed { tokens::DANGER } else { tokens::SUBTLE })))
                .into_element()
        }
    }
}

/// La lista de hilos: filas planas con la inicial del flujo, el estado y el último mensaje.
pub fn list(list: Vec<Thread>, open: Signal<Option<String>>, text_sig: Signal<String>) -> View {
    if list.is_empty() {
        return view().items_center().gap_2().py(px(48.0)).px_4().child(text("No threads yet").text_size(px(15.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))).child(
            text("Flows with agents (Condx → Workflows) open one per run. You can follow them and answer here.")
                .text_size(px(13.0))
                .line_height(px(19.0))
                .text_center()
                .text_color(c(tokens::MUTED)),
        );
    }
    let mut col = view().border_1().border_color(c(tokens::CARD_BORDER)).rounded(px(10.0)).overflow_hidden();
    let n = list.len();
    for (ix, t) in list.into_iter().enumerate() {
        let id = t.id.clone();
        let last = t.items.iter().rev().find(|i| i["text"].as_str().is_some_and(|s| !s.trim().is_empty()));
        let preview = last
            .map(|i| {
                let who = match i["kind"].as_str() {
                    Some("user") => "You: ".to_string(),
                    Some("agent" | "reply") => format!("{}: ", i["author"].as_str().unwrap_or("Agent")),
                    _ => String::new(),
                };
                format!("{who}{}", i["text"].as_str().unwrap_or_default().lines().find(|l| !l.trim().is_empty()).unwrap_or(""))
            })
            .unwrap_or_default();
        let attention = t.waiting_answer || t.status == "waiting-approval";
        let mut row = view()
            .flex_row()
            .gap_3()
            .px_3()
            .py(px(12.0))
            .bg(c(tokens::CARD))
            .active(|s| s.bg(c(tokens::ACTIVE)))
            .on_press(move || {
                text_sig.set(String::new());
                open.set(Some(id.clone()));
            })
            .child(avatar(&t.title, 38.0))
            .child(
                view()
                    .flex_1()
                    .gap(px(3.0))
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .child(view().flex_1().child(text(t.title.clone()).number_of_lines(1u32).text_size(px(14.5)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))))
                            .child(text(short_ago(t.updated_at)).text_size(px(11.5)).text_color(c(tokens::SUBTLE))),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .child(theme::dot(thread_health(&t.status)))
                            .child(text(thread_state(&t)).text_size(px(12.0)).text_color(c(if attention { tokens::WARNING } else { tokens::MUTED }))),
                    )
                    .child(text(preview).number_of_lines(2u32).text_size(px(13.0)).line_height(px(18.0)).text_color(c(tokens::MUTED))),
            );
        if ix + 1 < n {
            row = row.border_b_1().border_color(c(tokens::CARD_BORDER));
        }
        col = col.child(row);
    }
    col
}

/// Un hilo abierto: cabecera con volver, los mensajes (bajan solos al final) y el campo fijo abajo.
pub fn screen(t: Thread, open: Signal<Option<String>>, text_sig: Signal<String>, send: impl Fn(String, String) + Clone + 'static) -> View {
    let scroller = element_ref();
    let last_question = t.items.iter().rposition(|i| i["kind"] == "question");
    let id = t.id.clone();
    let quick = {
        let (send, id) = (send.clone(), id.clone());
        move |label: &'static str| -> View {
            let (send, id) = (send.clone(), id.clone());
            view()
                .px_4()
                .h(px(32.0))
                .justify_center()
                .rounded(px(16.0))
                .border_1()
                .border_color(c(tokens::WARNING))
                .active(|s| s.bg(c(tokens::ACTIVE)))
                .on_press(move || send(id.clone(), label.to_lowercase()))
                .child(text(label).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG)))
        }
    };
    let mut items = view().gap(px(14.0)).px_4().py_4();
    for (ix, it) in t.items.iter().enumerate().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
        items = items.child(message(it, Some(ix) == last_question && t.waiting_answer, &quick));
    }
    if t.status == "running" && !t.items.last().is_some_and(|i| i["status"] == "running") {
        items = items.child(view().flex_row().gap_2().items_center().child(theme::dot(Health::Warn)).child(muted("Working…")));
    }

    // Lo que hizo el daemon con tu último mensaje.
    let delivery = match (&t.result, t.waiting) {
        (_, true) => Some((Health::Unknown, "Sending… the Mac answers in the thread".to_string())),
        (Some(r), false) if r.starts_with("rejected") => Some((Health::Bad, format!("Couldn't deliver it: {}", r.trim_start_matches("rejected: ")))),
        (Some(r), false) if r == "sent to question" => Some((Health::Ok, "Answer sent".to_string())),
        (Some(r), false) => Some((Health::Ok, r.replacen("sent to", "Sent to", 1))),
        _ => None,
    };
    let placeholder: &'static str = if t.waiting_answer { "Answer the question…" } else { "Message the agents…" };
    let mut chips = view().flex_row().gap_2().px_3().pt_2();
    for p in &t.participants {
        let handle = p["node"].as_str().unwrap_or_default().to_string();
        let name = p["name"].as_str().unwrap_or(&handle).to_string();
        chips = chips.child(
            view()
                .flex_row()
                .gap(px(6.0))
                .items_center()
                .pl(px(4.0))
                .pr(px(10.0))
                .h(px(28.0))
                .rounded(px(14.0))
                .border_1()
                .border_color(c(tokens::CARD_BORDER))
                .active(|s| s.bg(c(tokens::ACTIVE)))
                .on_press(move || text_sig.set(format!("@{handle} ")))
                .child(avatar(&name, 20.0))
                .child(text(format!("@{name}")).text_size(px(12.5)).text_color(c(tokens::FG))),
        );
    }
    let do_send = move || {
        let msg = text_sig.get().trim().to_string();
        if msg.is_empty() {
            return;
        }
        text_sig.set(String::new());
        send(id.clone(), msg);
        scroller.scroll_to_end(true);
    };
    let do_send2 = do_send.clone();

    view()
        .flex_1()
        .bg(c(tokens::PANEL))
        // Sobre el teclado (Android no achica la ventana): lo que tapa, menos el margen seguro de abajo
        // que ya deja el safe area.
        .inset_padding(|| EdgeInsets { bottom: (keyboard_height() - safe_area().bottom).max(Points::ZERO), ..EdgeInsets::default() })
        // Cabecera: volver, el título y el estado.
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .pr_4()
                .h(px(56.0))
                .border_b_1()
                .border_color(c(tokens::CARD_BORDER))
                .bg(c(tokens::GROUND))
                .child(
                    view()
                        .w(px(44.0))
                        .h(px(56.0))
                        .items_center()
                        .justify_center()
                        .active(|s| s.opacity(0.5))
                        .on_press(move || {
                            text_sig.set(String::new());
                            open.set(None);
                        })
                        .child(text("‹").text_size(px(30.0)).text_color(c(tokens::FG))),
                )
                .child(avatar(&t.title, 32.0))
                .child(
                    view()
                        .flex_1()
                        .pl_2()
                        .gap(px(2.0))
                        .child(text(t.title.clone()).number_of_lines(1u32).text_size(px(14.5)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG)))
                        .child(view().flex_row().gap_2().items_center().child(theme::dot(thread_health(&t.status))).child(text(thread_state(&t)).text_size(px(11.5)).text_color(c(tokens::MUTED)))),
                ),
        )
        .child(scroll().flex_1().element_ref(&scroller).on_layout(move |_: &Bounds<Points>| scroller.scroll_to_end(false)).child(items))
        // El campo, fijo abajo: a quién mencionar, qué pasó con lo último y escribir.
        .child(
            view()
                .border_t_1()
                .border_color(c(tokens::CARD_BORDER))
                .bg(c(tokens::GROUND))
                .child(if t.participants.is_empty() { view().into_element() } else { scroll().horizontal().child(chips).into_element() })
                .child(match delivery {
                    Some((h, label)) => view().px_4().pt_2().child(theme::status_tag(h, label)).into_element(),
                    None => view().into_element(),
                })
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .px_3()
                        .py_2()
                        .child(
                            view().flex_1().child(
                                input()
                                    .value(move || text_sig.get())
                                    .placeholder(placeholder)
                                    .placeholder_color(move || now(tokens::SUBTLE))
                                    .selection_color(move || now(tokens::ACCENT))
                                    .on_change(move |s: &str| text_sig.set(s.to_string()))
                                    .return_key_type(ReturnKeyType::Send)
                                    .on_submit(move |_: &str| do_send2())
                                    .px_4()
                                    .h(px(42.0))
                                    .rounded(px(21.0))
                                    .border_1()
                                    .border_color(c(tokens::CARD_BORDER))
                                    .bg(c(tokens::PANEL))
                                    .text_color(c(tokens::FG))
                                    .text_size(px(14.5))
                                    .font_family(theme::UI_FONT),
                            ),
                        )
                        .child(dynamic(move || {
                            let empty = text_sig.get().trim().is_empty();
                            let do_send = do_send.clone();
                            view()
                                .w(px(42.0))
                                .h(px(42.0))
                                .rounded(px(21.0))
                                .items_center()
                                .justify_center()
                                .bg(c(if empty { tokens::HOVER } else { tokens::ACCENT }))
                                .active(|s| s.opacity(0.8))
                                .on_press(move || do_send())
                                .child(text("↑").text_size(px(20.0)).font_weight(FontWeight::SEMIBOLD).text_color(c(if empty { tokens::SUBTLE } else { tokens::ON_ACCENT })))
                                .into_element()
                        })),
                ),
        )
}
