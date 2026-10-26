//! Los hilos (F6) como un chat: la lista de conversaciones y, abierta una, sus mensajes a pantalla
//! completa con el campo para escribir fijo abajo. Los agentes hablan en burbujas con su inicial;
//! lo tuyo va a la derecha en el amarillo de la marca; el error, la pregunta del flujo y lo que
//! espera aprobación son bloques con una franja de color; lo demás, una línea centrada y apagada.

use nsh_core::ui::prelude::*;
use serde_json::Value;

use std::rc::Rc;
use std::time::Duration;

use crate::live;
use crate::sync::{Approval, Thread};
use crate::theme::{self, c, muted, now, tokens, Health};

pub fn thread_health(status: &str) -> Health {
    match status {
        "running" | "waiting-approval" | "waiting-answer" => Health::Warn,
        "succeeded" | "done" => Health::Ok,
        "failed" | "stopped" => Health::Bad,
        _ => Health::Unknown,
    }
}

/// El estado real de un hilo, en una palabra y su color: lo que pasa ahora (un agente trabaja, algo
/// espera en cola, te toca) antes que cómo terminó lo último.
pub fn state_of(t: &Thread) -> (Health, &'static str) {
    if t.items.iter().any(|i| i["status"] == "queued") {
        return (Health::Warn, "Queued");
    }
    if t.items.iter().any(|i| i["live"] == true) || t.status == "running" {
        return (Health::Warn, "Working");
    }
    if t.waiting_answer || t.status == "waiting-answer" {
        return (Health::Warn, "Needs your answer");
    }
    match t.status.as_str() {
        "waiting-approval" => (Health::Warn, "Needs your approval"),
        "done" => (Health::Ok, "Replied"),
        "succeeded" => (Health::Ok, "Done"),
        "failed" => (Health::Bad, "Failed"),
        "stopped" => (Health::Bad, "Stopped"),
        "cancelled" => (Health::Unknown, "Cancelled"),
        _ => (Health::Unknown, "New"),
    }
}

/// Los agentes de un hilo, uno detrás de otro (hasta tres): con más de uno se apilan con un borde del
/// color del fondo; con uno, su avatar solo; sin ninguno, la inicial del título.
/// Los de un hilo en una fila, para la lista: hasta dos caras chicas un poco encimadas y «+N».
fn faces_inline(t: &Thread, size: f32) -> AnyElement {
    let people: Vec<(String, String)> = t.participants.iter().filter_map(|p| Some((p["name"].as_str()?.to_string(), p["logo"].as_str().unwrap_or("").to_string()))).collect();
    if people.is_empty() {
        return avatar(&t.title, size).into_element();
    }
    let mut row = view().flex_row().items_center().flex_shrink_0();
    for (ix, (name, logo)) in people.iter().take(2).enumerate() {
        let ring = view().w(px(size + 2.0)).h(px(size + 2.0)).rounded(px(size / 2.0 + 1.0)).items_center().justify_center().bg(c(tokens::CARD)).child(face(name, logo, size));
        row = row.child(if ix == 0 { ring } else { ring.ml(px(-size * 0.35)) });
    }
    if people.len() > 2 {
        row = row.child(text(format!("+{}", people.len() - 2)).text_size(px(11.0)).text_color(c(tokens::SUBTLE)).ml(px(3.0)));
    }
    row.into_element()
}

/// El estado de un hilo, discreto (igual en la lista y en el encabezado): un punto y la palabra solo
/// cuando pide algo (ámbar), trabaja o algo salió mal (rojo); «Replied»/«Done» no se dicen (es lo
/// normal). Después, dónde trabaja.
fn status_line(health: Health, state: &str, t: &Thread) -> View {
    let quiet = matches!(state, "Replied" | "Done" | "New");
    let mut row = view().flex_row().gap(px(6.0)).items_center();
    if !quiet {
        let color = match health {
            Health::Bad => tokens::DANGER,
            Health::Warn => tokens::WARNING,
            _ => tokens::MUTED,
        };
        row = row.child(theme::dot(health)).child(text(crate::i18n::t(state)).text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(c(color)));
        if t.project.is_some() || t.env.is_some() {
            row = row.child(text("·").text_size(px(11.5)).text_color(c(tokens::SUBTLE)));
        }
    }
    row.child(view().flex_1().child(where_tag(t)))
}

/// En qué proyecto y entorno trabaja el hilo: «heskala-erp · dev» (producción en rojo).
fn where_tag(t: &Thread) -> AnyElement {
    let label = match (&t.project, &t.env) {
        (Some(p), Some(e)) => format!("{p} · {e}"),
        (Some(p), None) => p.clone(),
        (None, Some(e)) => e.clone(),
        (None, None) => return view().into_element(),
    };
    let prod = t.env.as_deref().is_some_and(|e| e.contains("prod"));
    text(label).number_of_lines(1u32).text_size(px(11.5)).font_family(theme::MONO_FONT).text_color(c(if prod { tokens::DANGER } else { tokens::SUBTLE })).into_element()
}

/// Qué hace ahora el agente que trabaja en el hilo («Reviewer · Reading the code…»), si hay uno.
fn activity_of(t: &Thread) -> Option<String> {
    let it = t.items.iter().rev().find(|i| i["live"] == true)?;
    let who = it["author"].as_str().unwrap_or("Agent");
    let doing = if it["text"].as_str().is_some_and(|s| !s.trim().is_empty()) && it["activity"] == "Thinking…" { "Writing…" } else { it["activity"].as_str().unwrap_or("Thinking…") };
    Some(format!("{who} · {}", crate::i18n::t(doing)))
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

/// Un adjunto que se está preparando para el próximo mensaje.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingFile {
    pub local: u64,
    pub name: String,
    pub mime: String,
    /// `image`, `audio` o `file`.
    pub kind: String,
    pub seconds: Option<f64>,
    /// `None` mientras sube; `Some(Ok(id))` listo; `Some(Err(..))` falló.
    pub uploaded: Option<Result<String, String>>,
}

/// Lo que el campo de escribir necesita de la app: los adjuntos pendientes, la grabación y qué hacer.
#[derive(Clone)]
pub struct Composer {
    pub files: Signal<Vec<PendingFile>>,
    /// Segundos grabados mientras graba una nota de voz.
    pub recording: Signal<Option<f64>>,
    pub menu: Signal<bool>,
    /// Las aprobaciones (para las tarjetas dentro del hilo) y cómo decidirlas.
    pub approvals: Signal<Vec<Approval>>,
    pub decide: Decide,
    /// El mensaje que se está respondiendo (deslizando o manteniendo presionado).
    pub reply: Signal<ReplyTo>,
    /// El catálogo de agentes: (id, nombre, marca del proveedor).
    pub catalog: Signal<Vec<(String, String, String)>>,
    pub photo: Rc<dyn Fn()>,
    pub mic: Rc<dyn Fn()>,
    /// Termina la grabación: `true` la manda, `false` la descarta.
    pub stop: Rc<dyn Fn(bool)>,
    /// Vuelve a la lista (cierra el teclado antes).
    pub close: Rc<dyn Fn()>,
    /// Quién contesta: la Mac, el sitio en la nube (la Mac no está) o nadie (sin conexión).
    pub presence: Signal<Presence>,
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Presence {
    #[default]
    Mac,
    Cloud,
    Offline,
}

/// El ícono chico de quién contesta, en la esquina del cuadro de escribir (sin palabras).
pub fn presence_icon(p: Presence) -> AnyElement {
    let (icon, color) = match p {
        Presence::Mac => (theme::LAPTOP, tokens::SUCCESS),
        Presence::Cloud => (theme::CLOUD, tokens::INFO),
        Presence::Offline => (theme::CLOUD, tokens::DANGER),
    };
    theme::icon(icon, color, 14.0).into_element()
}

fn file_icon(kind: &str) -> &'static str {
    match kind {
        "image" => "▣",
        "audio" => "♪",
        _ => "▤",
    }
}

fn clock(s: f64) -> String {
    format!("{}:{:02}", s as u64 / 60, s as u64 % 60)
}

/// Un chip de adjunto (en el campo y en las burbujas).
fn file_chip(kind: &str, label: String, tone: (u32, u32)) -> View {
    view()
        .flex_row()
        .gap(px(6.0))
        .items_center()
        .px_2()
        .h(px(26.0))
        .rounded(px(6.0))
        .bg(c(tokens::HOVER))
        .child(text(file_icon(kind)).text_size(px(12.0)).text_color(c(tokens::MUTED)))
        .child(text(label).number_of_lines(1u32).text_size(px(12.0)).text_color(c(tone)))
}

/// Lo que escribió la persona, con sus adjuntos como chips: `[image: x]`, `[voice 0:12] texto`,
/// `[file: x]` (con su contenido en un bloque, que no se muestra) los arma el daemon.
fn user_text(body: &str) -> View {
    let mut chips = view().flex_row().flex_wrap().gap(px(6.0));
    let mut n_chips = 0;
    let mut lines: Vec<String> = Vec::new();
    let mut skip_fence = false;
    let mut in_fence = false;
    for line in body.lines() {
        if in_fence {
            if line.trim_start().starts_with("```") {
                in_fence = false;
            }
            continue;
        }
        if skip_fence && line.trim_start().starts_with("```") {
            in_fence = true;
            skip_fence = false;
            continue;
        }
        skip_fence = false;
        if let Some(name) = line.strip_prefix("[image: ").and_then(|l| l.strip_suffix(']')) {
            chips = chips.child(file_chip("image", name.to_string(), tokens::FG));
            n_chips += 1;
        } else if let Some(rest) = line.strip_prefix("[voice") {
            let (head, said) = rest.split_once(']').unwrap_or((rest, ""));
            chips = chips.child(file_chip("audio", crate::i18n::tr(format!("Voice note{}", head), format!("Nota de voz{}", head)), tokens::FG));
            n_chips += 1;
            if !said.trim().is_empty() {
                lines.push(said.trim().to_string());
            }
        } else if let Some(name) = line.strip_prefix("[file: ").and_then(|l| l.strip_suffix(']')) {
            chips = chips.child(file_chip("file", name.to_string(), tokens::FG));
            n_chips += 1;
            skip_fence = true;
        } else if let Some(name) = line.strip_prefix("[file: ").and_then(|l| l.split(',').next()) {
            chips = chips.child(file_chip("file", name.to_string(), tokens::FG));
            n_chips += 1;
        } else {
            lines.push(line.to_string());
        }
    }
    let said = lines.join("\n").trim().to_string();
    let mut col = view().gap(px(6.0));
    if n_chips > 0 {
        col = col.child(chips);
    }
    if !said.is_empty() {
        col = col.child(text(said).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::FG)));
    }
    col
}

/// Decidir una aprobación desde el hilo (la misma decisión firmada que en la pestaña Aprobaciones).
pub type Decide = Rc<dyn Fn(String, bool, Option<String>)>;

/// La tarjeta de una aprobación dentro del hilo: su estado y, si espera, aprobar o rechazar. Se
/// redibuja sola cuando cambian las aprobaciones (no el hilo entero).
fn approval_card(job: String, body: String, approvals: Signal<Vec<Approval>>, decide: Decide) -> View {
    view().child(dynamic(move || {
        let found = approvals.get().into_iter().find(|a| a.job_id == job);
        let mut b = view().gap_2().child(text(body.clone()).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::FG)));
        let Some(a) = found else {
            return banner(tokens::SUBTLE, "Approval", b.child(muted("Decided, or no longer waiting."))).into_element();
        };
        let prod = a.level == "reinforced" || a.env_kind == "cloud-prod";
        let typed = a.level == "typed" && !prod;
        let waiting = a.status == "waiting-approval" && a.decision.is_none();
        let state = match (&a.decision, a.pending, a.phone_result.as_deref()) {
            (Some(_), true, _) => Some((Health::Unknown, "Sending your decision…".to_string())),
            (Some(_), false, Some("approved")) => Some((Health::Ok, "Approved from this phone".to_string())),
            (Some(_), false, Some("rejected the job")) => Some((Health::Bad, "Rejected from this phone".to_string())),
            (Some(_), false, Some("needs-mac")) => Some((Health::Warn, "Waiting for Touch ID on the Mac".to_string())),
            (Some(_), false, Some(r)) => Some((Health::Bad, crate::i18n::tr(format!("The Mac didn't apply it: {}", r.trim_start_matches("rejected: ")), format!("La Mac no lo aplicó: {}", r.trim_start_matches("rejected: "))))),
            (Some(d), false, None) => Some((Health::Unknown, crate::i18n::tr(format!("You chose {d} · the Mac checks it"), format!("Elegiste {d} · la Mac lo revisa")))),
            (None, _, _) if !waiting => Some((Health::Unknown, crate::i18n::tr(format!("Done: {}", a.status), format!("Terminado: {}", a.status)))),
            _ => None,
        };
        b = b.child(muted(format!("{} · {}{}", a.env, a.level, if prod { " · production: Touch ID on the Mac" } else { "" })));
        if let Some((h, l)) = state {
            b = b.child(theme::status_tag(h, l));
        }
        if waiting {
            let (d1, d2, j1, j2, env) = (decide.clone(), decide.clone(), a.job_id.clone(), a.job_id.clone(), a.env.clone());
            b = if typed {
                b.child(theme::slide_to_confirm("Slide to approve", move || d1(j1.clone(), true, Some(env.clone()))))
                    .child(view().flex_row().justify_end().child(theme::small_button("Reject", theme::Variant::Danger, move || d2(j2.clone(), false, None))))
            } else {
                b.child(
                    view()
                        .flex_row()
                        .gap_2()
                        .justify_end()
                        .child(theme::small_button("Reject", theme::Variant::Danger, move || d2(j2.clone(), false, None)))
                        .child(theme::small_button(if prod { "Propose" } else { "Approve" }, theme::Variant::Primary, move || d1(j1.clone(), true, None))),
                )
            };
        }
        banner(if waiting { tokens::WARNING } else { tokens::SUBTLE }, if waiting { "Needs your approval" } else { "Approval" }, b).into_element()
    }))
}

/// Lo que se responde: (a quién mencionar, si se puede; su nombre; un trozo del mensaje).
pub type ReplyTo = Option<(Option<String>, String, String)>;

/// Separa lo que se escribió de la cita que lo acompaña (la agrega el teléfono al responder).
fn split_quote(body: &str) -> (String, Option<String>) {
    match body.split_once("\n\n> Replying to ") {
        Some((msg, quote)) => (msg.to_string(), Some(quote.lines().map(|l| l.trim_start_matches("> ")).collect::<Vec<_>>().join(" "))),
        None => (body.to_string(), None),
    }
}

/// Deslizar a la derecha para responder (como en WhatsApp): la burbuja sigue al dedo y, pasado un
/// tramo, queda citada arriba del campo.
fn swipe_to_reply(content: View, reply: Signal<ReplyTo>, to: ReplyTo) -> View {
    let x = animated_value(0.0);
    let shift = x.interpolate(&[0.0, 72.0], &[0.0, 72.0]);
    let hint = x.interpolate(&[0.0, 48.0], &[0.0, 1.0]);
    view()
        .child(view().absolute().left(px(4.0)).top(px(0.0)).bottom(px(0.0)).justify_center().bind_opacity(&hint).child(text("↩").text_size(px(18.0)).text_color(c(tokens::MUTED))))
        .child(
            view()
                .bind_translate_x(&shift)
                // Solo hacia la derecha: hacia la izquierda falla y el gesto queda para lo de adentro (el
                // código se desplaza de lado).
                .claim_touches(TouchClaim::on_pan(PanActivation { active_offset_x: Some(px(14.0)), active_offset_y: None, fail_offset_x: Some(px(-6.0)), fail_offset_y: Some(px(14.0)) }))
                .pan_values(Some(&x), None)
                .on_pan_end(move |e: &PanEvent| {
                    if e.value_x.unwrap_or(0.0) >= 48.0 {
                        reply.set(to.clone());
                    }
                    let _ = x.spring(0.0, Spring::default());
                })
                .child(content),
        )
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

/// El avatar de un agente: el logo de su proveedor si se conoce; si no, su inicial.
fn face(name: &str, logo: &str, size: f32) -> AnyElement {
    crate::logos::avatar(logo, size).unwrap_or_else(|| avatar(name, size).into_element())
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
    theme::short(ms)
}

/// El texto de un agente: la prosa arriba y los campos de su resultado (`clave: valor`) como filas.
/// Lo que el agente escribe ahora: el texto aparece a su paso con el cursor al final; sin texto
/// todavía (o mientras usa una herramienta), qué hace, con un brillo que lo recorre.
fn live_body(name: &str, it: &Value) -> AnyElement {
    let text_now = it["text"].as_str().unwrap_or("").to_string();
    let activity = it["activity"].as_str().unwrap_or("Thinking…").to_string();
    let key = format!("{name}:{}", it["at"]);
    let thinking = activity == "Thinking…";
    dynamic(move || {
        global::<live::Tick>().get();
        let shown = live::revealed(&key, &text_now);
        let mut col = view().gap(px(6.0));
        if !shown.is_empty() {
            col = col.child(view().px_3().py(px(10.0)).rounded(px(14.0)).rounded_tl(px(4.0)).bg(c(tokens::HOVER)).child(agent_body(&format!("{shown}{}", live::CARET))));
        }
        // Escribiendo: el cursor basta; pensando o con una herramienta, se dice qué hace.
        if shown.is_empty() || !thinking {
            col = col.child(view().pl(px(2.0)).child(live::shimmer(&activity, 13.5)));
        }
        col.into_element()
    })
    .into_element()
}

/// Baja al último mensaje si se sigue el final; si no, vuelve a donde se estaba leyendo (el hilo se
/// vuelve a armar en cada sincronización).
fn follow(scroller: ElementRef, animated: bool) {
    if live::following() {
        scroller.scroll_to_end(animated);
    } else {
        scroller.scroll_to(Point { x: Points(0.0), y: Points(live::offset()) }, false);
    }
}

fn agent_body(body: &str) -> View {
    let mut prose: Vec<String> = Vec::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut fence: Option<Vec<String>> = None;
    for line in body.lines() {
        if let Some(code) = fence.as_mut() {
            if line.trim_start().starts_with("```") {
                // El JSON del resultado ya sale como campos: no se repite.
                let block = std::mem::take(code);
                let is_result = block.first().is_some_and(|l| l.trim_start().starts_with("```json")) && serde_json::from_str::<serde_json::Map<String, Value>>(&block[1..].join("\n")).is_ok();
                if !is_result {
                    prose.extend(block);
                    prose.push(line.to_string());
                }
                fence = None;
            } else {
                code.push(line.to_string());
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            fence = Some(vec![line.to_string()]);
            continue;
        }
        match line.split_once(": ") {
            Some((k, v)) if !k.is_empty() && k.len() <= 24 && k.chars().all(|c| c.is_ascii_lowercase() || c == '_') => {
                let v = v.trim();
                if !v.is_empty() {
                    fields.push((k.to_string(), if k == "commit" { v.chars().take(10).collect() } else { v.to_string() }));
                }
            }
            _ => prose.push(line.to_string()),
        }
    }
    if let Some(code) = fence {
        prose.extend(code);
    }
    // El resumen primero; lo demás en orden fijo (llegan en cualquier orden).
    fields.sort_by(|a, b| (a.0 != "summary", &a.0).cmp(&(b.0 != "summary", &b.0)));
    let mut col = view().gap_2();
    let prose = prose.join("\n");
    if !prose.trim().is_empty() {
        col = col.child(crate::md::render(prose.trim(), 14.0));
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

/// La marca de un mensaje enviado: una (✓, en la nube) o dos (✓✓, la leyó un agente), dibujadas como
/// íconos (el carácter de la fuente sale como «√»). Caja de 16×11, trazo redondeado.
fn check_mark(double: bool, color: (u32, u32)) -> View {
    let path = if double { "M1 6.2L4.4 9.5L11 2.5M6.4 8.4L7.6 9.5L14.2 2.5" } else { "M3 6.2L6.4 9.5L13 2.5" };
    let k = 0.9;
    let icon = canvas(move || Painter::new().scale(k, k).stroke_style(crate::theme::now(color)).line_width(1.3).line_cap(nsh_core::canvas::LineCap::Round).line_join(nsh_core::canvas::LineJoin::Round).stroke_svg(path))
        .w(px(16.0 * k))
        .h(px(11.0 * k));
    view().child(icon)
}

fn message(it: &Value, last_question: bool, quick: &dyn Fn(&'static str) -> View, approvals: Signal<Vec<Approval>>, decide: &Decide, reply: Signal<ReplyTo>, handles: &[(String, String, String)]) -> AnyElement {
    let body = it["text"].as_str().unwrap_or_default().trim().to_string();
    let at = it["at"].as_i64().unwrap_or(0);
    match it["kind"].as_str().unwrap_or("system") {
        "user" => {
            let to = it["to"].as_str().filter(|t| !t.is_empty()).map(|t| format!("to {t} · ")).unwrap_or_default();
            let (body, quote) = split_quote(&body);
            let snippet: String = body.chars().take(90).collect();
            let mut bubble_inner = view().gap(px(6.0));
            if let Some(q) = quote {
                bubble_inner = bubble_inner.child(view().flex_row().child(view().w(px(3.0)).rounded(px(2.0)).bg(c(tokens::WARNING))).child(view().flex_1().pl_2().child(text(q).number_of_lines(2u32).text_size(px(12.5)).line_height(px(17.0)).text_color(c(tokens::MUTED)))));
            }
            let bubble = view()
                .items_end()
                .pl(px(56.0))
                .gap_1()
                .child(
                    view()
                        .px_3()
                        .py_2()
                        .rounded(px(14.0))
                        .rounded_br(px(4.0))
                        .bg(c(tokens::USER_BUBBLE))
                        .border_1()
                        .border_color(c(tokens::USER_BUBBLE_BORDER))
                        .child(bubble_inner.child(user_text(&body))),
                )
                // En cola: sale cuando un agente termina (ya trabajan tantos como el límite).
                .child(if it["status"] == "queued" {
                    text(crate::i18n::t("Queued · goes out when an agent is free")).text_size(px(11.0)).text_color(c(tokens::WARNING)).into_element()
                } else {
                    // ✓✓ antes de la hora, en el mismo gris (como Claude), cuando un agente ya lo leyó.
                    let mut row = view().flex_row().items_center().gap(px(4.0));
                    if it["mark"] == "read" {
                        row = row.child(check_mark(true, tokens::SUBTLE));
                    }
                    row.child(text(format!("{to}{}", short_ago(at))).text_size(px(11.0)).text_color(c(tokens::SUBTLE))).into_element()
                });
            let to = Some((None, "you".to_string(), snippet));
            let long = to.clone();
            swipe_to_reply(bubble.on_long_press(move || reply.set(long.clone())), reply, to).into_element()
        }
        "agent" | "reply" | "external" => {
            let external = it["kind"] == "external";
            let name = it["author"].as_str().unwrap_or("Agent").to_string();
            let handle = handles.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(&name)).map(|(_, h, _)| h.clone());
            let logo = it["logo"].as_str().map(str::to_string).or_else(|| handles.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(&name)).map(|(_, _, l)| l.clone())).unwrap_or_default();
            let snippet: String = body.lines().find(|l| !l.trim().is_empty() && !l.trim_start().starts_with("```")).unwrap_or("").replace("**", "").replace('`', "").chars().take(90).collect();
            // El avatar pequeño junto al nombre y la burbuja debajo, a todo el ancho (sin la columna
            // vacía bajo el avatar).
            let bubble = view()
                .gap(px(6.0))
                .pr(px(24.0))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .child(face(&name, &logo, 16.0))
                        .child(text(name.clone()).text_size(px(13.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG)))
                        // Un agente de fuera (por el MCP): lo que dice es suyo, no de Condx.
                        .child(if external { view().px(px(5.0)).h(px(16.0)).justify_center().rounded(px(4.0)).border_1().border_color(c(tokens::CARD_BORDER)).child(text("external").text_size(px(10.0)).text_color(c(tokens::MUTED))).into_element() } else { view().into_element() })
                        .child(text(short_ago(at)).text_size(px(11.0)).text_color(c(tokens::SUBTLE))),
                )
                .child(if it["live"] == true {
                    live_body(&name, it)
                } else {
                    view().px_3().py(px(10.0)).rounded(px(14.0)).rounded_tl(px(4.0)).bg(c(tokens::HOVER)).child(agent_body(&body)).into_element()
                });
            // Mantener presionado también responde; con código adentro es la única forma (el gesto
            // lateral es del código o de la tabla, que se desplazan).
            let to = Some((handle, name, snippet));
            let long = to.clone();
            let bubble = bubble.on_long_press(move || reply.set(long.clone()));
            if body.contains("```") || crate::md::has_table(&body) {
                bubble.into_element()
            } else {
                swipe_to_reply(bubble, reply, to).into_element()
            }
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
        // Lo que el daemon le pasó al agente al abrir el hilo (un incidente): corto, apagado.
        "context" => banner(
            tokens::SUBTLE,
            "Context given to the agent",
            view().child(text(body.lines().filter(|l| !l.trim().is_empty()).skip(1).collect::<Vec<_>>().join("\n")).number_of_lines(4u32).text_size(px(12.5)).line_height(px(18.0)).text_color(c(tokens::MUTED))),
        )
        .into_element(),
        "approval" => match it["job"].as_str() {
            Some(job) => approval_card(job.to_string(), body, approvals, decide.clone()).into_element(),
            None => banner(tokens::WARNING, "Needs your approval", view().child(text(body).text_size(px(14.0)).text_color(c(tokens::FG)))).into_element(),
        },
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
/// Mientras llega la primera lista: filas grises con la forma de un hilo.
fn skeleton() -> View {
    let bar = |w: f32, h: f32| view().w(px(w)).h(px(h)).rounded(px(h / 2.0)).bg(c(tokens::HOVER));
    let mut col = view().border_1().border_color(c(tokens::CARD_BORDER)).rounded(px(10.0)).overflow_hidden();
    for ix in 0..5 {
        let row = view()
            .gap(px(9.0))
            .px_3()
            .py(px(14.0))
            .bg(c(tokens::CARD))
            .child(view().flex_row().gap_2().items_center().child(view().w(px(18.0)).h(px(18.0)).rounded(px(9.0)).bg(c(tokens::HOVER))).child(bar(180.0 + (ix % 3) as f32 * 30.0, 12.0)))
            .child(bar(70.0, 16.0))
            .child(bar(240.0 - (ix % 2) as f32 * 50.0, 10.0));
        col = col.child(if ix > 0 { row.border_t_1().border_color(c(tokens::CARD_BORDER)) } else { row });
    }
    col
}

pub fn list(list: Vec<Thread>, open: Signal<Option<String>>, text_sig: Signal<String>, loaded: bool, shown: usize) -> View {
    if list.is_empty() && !loaded {
        return skeleton();
    }
    let more = list.len() > shown;
    let list: Vec<Thread> = list.into_iter().take(shown).collect();
    if list.is_empty() {
        return view().items_center().gap_2().py(px(48.0)).px_4().child(text(crate::i18n::t("No threads yet")).text_size(px(15.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))).child(
            text(crate::i18n::t("Flows with agents (Condx → Workflows) open one per run. You can follow them and answer here."))
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
                // Sin las marcas del markdown (en la vista previa no se estiliza).
                let line = i["text"].as_str().unwrap_or_default().lines().find(|l| !l.trim().is_empty() && !l.trim_start().starts_with("```")).unwrap_or("");
                format!("{who}{}", line.replace("**", "").replace('`', "").trim_start_matches("#").trim())
            })
            .unwrap_or_default();
        let (health, state) = state_of(&t);
        let queued = t.items.iter().filter(|i| i["status"] == "queued").count();
        let activity = activity_of(&t);
        // Arriba el título a todo el ancho con la hora; abajo lo último (o lo que hace ahora el agente,
        // con el brillo) y el estado, con los agentes en la esquina de abajo a la derecha.
        let last_line = match (activity, queued) {
            (Some(doing), _) => dynamic(move || {
                global::<live::Tick>().get();
                live::shimmer(&doing, 13.0).into_element()
            })
            .into_element(),
            (None, n) if n > 0 => text(match (n, crate::i18n::spanish()) { (1, false) => "1 message waits for a free agent".to_string(), (_, false) => format!("{n} messages wait for a free agent"), (1, true) => "1 mensaje espera un agente libre".to_string(), (_, true) => format!("{n} mensajes esperan un agente libre") }).text_size(px(13.0)).text_color(c(tokens::MUTED)).into_element(),
            _ => text(preview).number_of_lines(2u32).text_size(px(13.0)).line_height(px(18.0)).text_color(c(tokens::MUTED)).into_element(),
        };
        let mut row = view()
            .gap(px(4.0))
            .px_3()
            .py(px(12.0))
            .bg(c(tokens::CARD))
            .active(|s| s.bg(c(tokens::ACTIVE)))
            .on_press(move || {
                text_sig.set(String::new());
                live::reset_follow();
                open.set(Some(id.clone()));
            })
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
                    .gap_3()
                    .items_end()
                    .child(view().flex_1().gap(px(6.0)).child(last_line).child(status_line(health, state, &t)))
                    .child(view().pb(px(1.0)).child(faces_inline(&t, 18.0))),
            );
        if ix + 1 < n {
            row = row.border_b_1().border_color(c(tokens::CARD_BORDER));
        }
        col = col.child(row);
    }
    // Hay más hilos: aparecen al seguir bajando.
    if more {
        col = col.child(view().items_center().py(px(14.0)).bg(c(tokens::CARD)).border_t_1().border_color(c(tokens::CARD_BORDER)).child(live::shimmer("Loading more…", 12.5)));
    }
    col
}

/// Un hilo abierto: cabecera con volver, los mensajes (bajan solos al final) y el campo fijo abajo.
/// Los que están en el hilo: (nombre, @handle, marca).
fn handles_of(t: &Thread) -> Vec<(String, String, String)> {
    t.participants.iter().filter_map(|p| Some((p["name"].as_str()?.to_string(), p["node"].as_str()?.trim_start_matches("g-").to_string(), p["logo"].as_str().unwrap_or("").to_string()))).collect()
}

/// Un hilo abierto. Se monta una vez por hilo: lo que cambia con cada sincronización (la cabecera,
/// los mensajes, los chips) se vuelve a dibujar solo, y el campo queda montado (no pierde el foco ni
/// cierra el teclado mientras un agente escribe o al mandar).
pub fn screen(id: String, current: Rc<dyn Fn() -> Option<Thread>>, text_sig: Signal<String>, send: impl Fn(String, String) + Clone + 'static, composer: Composer) -> View {
    let owner = Owner::current();
    let (approvals, decide, reply) = (composer.approvals, composer.decide.clone(), composer.reply);
    let scroller = element_ref();
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
    let body = {
        let current = current.clone();
        move || -> AnyElement {
            let Some(t) = current() else { return view().into_element() };
            let handles = handles_of(&t);
            let last_question = t.items.iter().rposition(|i| i["kind"] == "question");
            let mut items = view().gap(px(14.0)).px_4().py_4();
            if t.id == "_new" && t.sending.is_none() {
                items = items.child(
                    view().items_center().gap_2().py(px(40.0)).px_4().child(text(crate::i18n::t("Start a conversation")).text_size(px(15.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))).child(
                        text(crate::i18n::t("Ask the agents about your project: they read your code and what Condx knows (environments, jobs, incidents, flows). You can attach a photo or send a voice note."))
                            .text_size(px(13.0))
                            .line_height(px(19.0))
                            .text_center()
                            .text_color(c(tokens::MUTED)),
                    ),
                );
            }
            // ✓✓ junto a la hora de tu último mensaje cuando un agente ya lo tomó.
            let last_user = t.items.iter().rposition(|i| i["kind"] == "user").filter(|_| t.read && t.sending.is_none());
            for (ix, it) in t.items.iter().enumerate().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
                let mut it = it.clone();
                if Some(ix) == last_user {
                    it["mark"] = "read".into();
                }
                items = items.child(message(&it, Some(ix) == last_question && t.waiting_answer, &quick, approvals, &decide, reply, &handles));
            }
            // Lo que acabas de mandar, al instante, hasta que el daemon lo contesta.
            if let Some(pending) = t.sending.clone() {
                items = items.child(
                    view()
                        .items_end()
                        .pl(px(56.0))
                        .gap_1()
                        .opacity(0.75)
                        .child(view().px_3().py_2().rounded(px(14.0)).rounded_br(px(4.0)).bg(c(tokens::USER_BUBBLE)).border_1().border_color(c(tokens::USER_BUBBLE_BORDER)).child(if pending.trim().is_empty() { text(crate::i18n::t("Attachment")).text_size(px(14.0)).text_color(c(tokens::MUTED)) } else { text(pending).text_size(px(14.0)).line_height(px(20.0)).text_color(c(tokens::FG)) }))
                        // Enviándose (todavía en el teléfono): gira; ya en la nube, esperando al agente: ✓.
                        .child(view().flex_row().items_center().gap(px(4.0)).child(if t.delivered { check_mark(false, tokens::SUBTLE) } else { live::spinner(10.0) }).child(text(crate::i18n::t("now")).text_size(px(11.0)).text_color(c(tokens::SUBTLE)))),
                );
            }
            // Con un agente escribiendo, su burbuja ya dice que trabaja.
            if t.status == "running" && !t.items.last().is_some_and(|i| i["status"] == "running" || i["live"] == true) {
                items = items.child(view().flex_row().gap_2().items_center().child(theme::dot(Health::Warn)).child(muted("Working…")));
            }
            items.on_layout(move |_: &Bounds<Points>| follow(scroller, true)).into_element()
        }
    };
    let placeholder = {
        let current = current.clone();
        move || if current().is_some_and(|t| t.waiting_answer) { crate::i18n::t("Answer the question…") } else { crate::i18n::t("Message the agents…") }
    };
    // El campo de escribir: los chips de agentes lo enfocan y ponen el cursor al final.
    let field = element_ref();
    let close = composer.close.clone();
    let remount = signal(0u32);
    // Los chips de a quién mencionar y qué pasó con lo último que mandaste (cambian con el hilo).
    let chips_row = {
        let current = current.clone();
        let catalog = composer.catalog;
        move || -> AnyElement {
            let Some(t) = current() else { return view().into_element() };
            let mut chips = view().flex_row().gap_2().px_3().pt_2();
            // Los del hilo y, después, los del catálogo que no están (se los llama con @ y entran a leer y responder).
            let mut people: Vec<(String, String, String, bool)> = handles_of(&t).iter().map(|(n, h, l)| (n.clone(), h.clone(), l.clone(), true)).collect();
            for (id, name, logo) in catalog.get() {
                if !people.iter().any(|(n, h, _, _)| h.eq_ignore_ascii_case(&id) || n.eq_ignore_ascii_case(&name)) {
                    people.push((name, id, logo, false));
                }
            }
            // Hay a quién mencionar (los del hilo o el catálogo; un hilo nuevo todavía no tiene a nadie).
            let anyone = !people.is_empty();
            for (name, handle, logo, member) in people {
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
                        .on_press(move || {
                            // A quién va: la mención al principio (reemplaza otra si ya había), sin borrar
                            // lo escrito; el campo queda con el cursor al final y el teclado abierto.
                            // Con una palabra a medio componer el teclado retiene el valor nuevo hasta que
                            // termina (y soltar el foco no basta): el campo se vuelve a montar con el texto.
                            let cur = text_sig.get();
                            let rest = if cur.starts_with('@') { cur.split_once(char::is_whitespace).map(|(_, r)| r.to_string()).unwrap_or_default() } else { cur };
                            let next = format!("@{handle} {}", rest.trim_start());
                            let end = next.encode_utf16().count() as u32;
                            text_sig.set(next);
                            remount.set(remount.get() + 1);
                            owner.run(|| {
                                spawn(async move {
                                    timer(Duration::from_millis(60)).await;
                                    field.focus();
                                    field.set_selection(end, end);
                                })
                            });
                        })
                        .child(face(&name, &logo, 20.0))
                        .child(text(format!("@{name}")).text_size(px(12.5)).text_color(c(if member { tokens::FG } else { tokens::MUTED }))),
                );
            }
            // Lo que hizo el daemon con tu último mensaje (entregado ya se ve en el hilo).
            let delivery = match (&t.result, t.waiting) {
                (Some(r), false) if r.starts_with("rejected") => Some((Health::Bad, crate::i18n::tr(format!("Couldn't deliver it: {}", r.trim_start_matches("rejected: ")), format!("No se pudo entregar: {}", r.trim_start_matches("rejected: "))))),
                _ => None,
            };
            view()
                .child(if !anyone { view().into_element() } else { scroll().horizontal().keyboard_taps(KeyboardTaps::Handled).child(chips).into_element() })
                .child(match delivery {
                    Some((h, label)) => view().px_4().pt_2().child(theme::status_tag(h, label)).into_element(),
                    None => view().into_element(),
                })
                .into_element()
        }
    };
    let files = composer.files;
    let do_send = move || {
        let mut msg = text_sig.get().trim().to_string();
        let pending = files.get();
        if pending.iter().any(|f| f.uploaded.is_none()) || (msg.is_empty() && !pending.iter().any(|f| matches!(f.uploaded, Some(Ok(_))))) {
            return;
        }
        // Respondiendo un mensaje: va al agente que lo escribió (si no se mencionó a otro) con la cita.
        if let Some((handle, name, snippet)) = reply.get() {
            if let Some(h) = handle.filter(|_| !msg.starts_with('@')) {
                msg = format!("@{h} {msg}");
            }
            msg = format!("{msg}\n\n> Replying to {name}: {snippet}");
            reply.set(None);
        }
        text_sig.set(String::new());
        send(id.clone(), msg);
        scroller.scroll_to_end(true);
    };

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
                        .w(px(34.0))
                        .h(px(56.0))
                        .items_center()
                        .justify_center()
                        .active(|s| s.opacity(0.5))
                        .on_press(move || close())
                        .child(text("‹").text_size(px(30.0)).text_color(c(tokens::FG))),
                )
                .child(dynamic({
                    let current = current.clone();
                    move || {
                        let Some(t) = current() else { return view().into_element() };
                        view()
                            .flex_1()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(if t.id == "_new" { view().w(px(4.0)).into_element() } else { faces_inline(&t, 18.0) })
                            .child(
                                view()
                                    .flex_1()
                                    .pl_2()
                                    .gap(px(2.0))
                                    .child(text(t.title.clone()).number_of_lines(1u32).text_size(px(14.5)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG)))
                                    .child(if t.id == "_new" {
                                        text(crate::i18n::t("The agents answer here")).text_size(px(11.5)).text_color(c(tokens::MUTED)).into_element()
                                    } else {
                                        {
                                            let (health, state) = state_of(&t);
                                            status_line(health, state, &t).into_element()
                                        }
                                    }),
                            )
                            .into_element()
                    }
                })),
        )
        // Al cambiar lo de adentro (llegó un mensaje, se abrió el teclado), baja al último, salvo que
        // se haya subido a leer: entonces queda donde estaba y aparece el botón para bajar.
        .child(
            view()
                .flex_1()
                .child(
                    scroll()
                        .flex_1()
                        .keyboard_taps(KeyboardTaps::Handled)
                        .element_ref(&scroller)
                        .on_scroll(move |e: &ScrollEvent| live::scrolled(f32::from(e.offset.y), f32::from(e.content_size.height), f32::from(e.size.height)))
                        .on_layout(move |_: &Bounds<Points>| follow(scroller, false))
                        .child(dynamic(body)),
                )
                .child(dynamic(move || {
                    if !global::<live::ShowDown>().get().0 {
                        return view().into_element();
                    }
                    view()
                        .absolute()
                        .left(px(0.0))
                        .right(px(0.0))
                        .bottom(px(12.0))
                        .items_center()
                        .child(
                            view()
                                .w(px(40.0))
                                .h(px(40.0))
                                .rounded(px(20.0))
                                .items_center()
                                .justify_center()
                                .border_1()
                                .border_color(c(tokens::CARD_BORDER))
                                .bg(c(tokens::GROUND))
                                .shadow_md()
                                .active(|s| s.opacity(0.8))
                                .on_press(move || {
                                    live::scrolled(0.0, 0.0, 0.0);
                                    scroller.scroll_to_end(true);
                                })
                                .child(theme::icon(theme::ARROW_DOWN, tokens::FG, 18.0)),
                        )
                        .into_element()
                })),
        )
        // El campo, fijo abajo: a quién mencionar, qué pasó con lo último y escribir (una tarjeta
        // flotante, como la de Claude).
        .child(
            view()
                .pb_2()
                // A qué se responde (deslizando un mensaje).
                .child(dynamic(move || match reply.get() {
                    Some((_, name, snippet)) => view()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .mx_3()
                        .mt_2()
                        .pl_2()
                        .rounded(px(6.0))
                        .bg(c(tokens::HOVER))
                        .child(view().w(px(3.0)).h(px(32.0)).rounded(px(2.0)).bg(c(tokens::WARNING)))
                        .child(view().flex_1().gap(px(1.0)).child(text(crate::i18n::tr(format!("Replying to {name}"), format!("Respondiendo a {name}"))).text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))).child(text(snippet).number_of_lines(1u32).text_size(px(12.0)).text_color(c(tokens::MUTED))))
                        .child(view().w(px(36.0)).h(px(36.0)).items_center().justify_center().on_press(move || reply.set(None)).child(text("×").text_size(px(18.0)).text_color(c(tokens::MUTED))))
                        .into_element(),
                    None => view().into_element(),
                }))
                .child(dynamic(chips_row))
                // Los adjuntos del próximo mensaje (suben enseguida; × los quita).
                .child(dynamic(move || {
                    let list = files.get();
                    if list.is_empty() {
                        return view().into_element();
                    }
                    let mut row = view().flex_row().gap_2().px_3().pt_2();
                    for f in list {
                        let (label, tone) = match &f.uploaded {
                            None => (format!("{} · uploading…", f.name), tokens::MUTED),
                            Some(Ok(_)) => (match f.seconds { Some(s) => format!("{} · {}", f.name, clock(s)), None => f.name.clone() }, tokens::FG),
                            Some(Err(e)) => (format!("{} · {e}", f.name), tokens::DANGER),
                        };
                        let local = f.local;
                        row = row.child(
                            file_chip(&f.kind, label, tone).pr(px(0.0)).child(
                                view().w(px(26.0)).h(px(26.0)).items_center().justify_center().on_press(move || files.set(files.get().into_iter().filter(|x| x.local != local).collect())).child(text("×").text_size(px(15.0)).text_color(c(tokens::MUTED))),
                            ),
                        );
                    }
                    scroll().horizontal().keyboard_taps(KeyboardTaps::Handled).child(row).into_element()
                }))
                // Qué adjuntar.
                .child(dynamic({
                    let photo = composer.photo.clone();
                    let menu = composer.menu;
                    move || {
                        if !menu.get() {
                            return view().into_element();
                        }
                        let photo = photo.clone();
                        let item = |label: &'static str, hint: &'static str, f: Rc<dyn Fn()>| {
                            view()
                                .flex_row()
                                .gap_3()
                                .items_center()
                                .px_3()
                                .h(px(44.0))
                                .active(|s| s.bg(c(tokens::ACTIVE)))
                                .on_press(move || {
                                    menu.set(false);
                                    f();
                                })
                                .child(text(label).text_size(px(14.0)).text_color(c(tokens::FG)))
                                .child(text(hint).text_size(px(12.0)).text_color(c(tokens::MUTED)))
                        };
                        view()
                            .mx_3()
                            .mt_2()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(c(tokens::CARD_BORDER))
                            .bg(c(tokens::PANEL))
                            .overflow_hidden()
                            .child(item("Take a photo", "the camera opens; agents that see images get it", photo))
                            .into_element()
                    }
                }))
                // Escribir, o la barra de la grabación mientras graba: la tarjeta con el texto arriba
                // y los botones abajo.
                .child(dynamic(move || {
                    remount.get();
                    let (stop_send, stop_drop) = (composer.stop.clone(), composer.stop.clone());
                    let presence = composer.presence;
                    let card = view()
                        .mx_3()
                        .mt_2()
                        .px(px(10.0))
                        .pt(px(8.0))
                        .pb(px(8.0))
                        .gap(px(6.0))
                        .rounded(px(22.0))
                        .border_1()
                        .border_color(c(tokens::CARD_BORDER))
                        .bg(c(tokens::RAISED))
                        // Quién contesta, en la esquina: laptop verde (la Mac), nube azul (el sitio en la
                        // nube) o nube roja (sin conexión).
                        .child(view().absolute().top(px(17.0)).right(px(16.0)).child(dynamic(move || presence_icon(presence.get()))));
                    let round = |size: f32| view().w(px(size)).h(px(size)).rounded(px(size / 2.0)).items_center().justify_center();
                    if let Some(secs) = composer.recording.get() {
                        return card
                            .child(
                                view()
                                    .flex_row()
                                    .gap_3()
                                    .items_center()
                                    .child(round(38.0).border_1().border_color(c(tokens::CARD_BORDER)).active(|s| s.bg(c(tokens::ACTIVE))).on_press(move || stop_drop(false)).child(text("×").text_size(px(19.0)).text_color(c(tokens::MUTED))))
                                    .child(view().flex_1().flex_row().gap_2().items_center().child(view().w(px(10.0)).h(px(10.0)).rounded(px(5.0)).bg(c(tokens::DANGER))).child(text(crate::i18n::tr(format!("Recording · {}", clock(secs)), format!("Grabando · {}", clock(secs)))).text_size(px(14.5)).text_color(c(tokens::FG))))
                                    .child(round(38.0).bg(c(tokens::ACCENT)).active(|s| s.opacity(0.8)).on_press(move || stop_send(true)).child(theme::icon(theme::ARROW_UP, tokens::ON_ACCENT, 20.0))),
                            )
                            .into_element();
                    }
                    let do_send = do_send.clone();
                    let mic = composer.mic.clone();
                    let menu = composer.menu;
                    let ready = files.get().iter().any(|f| matches!(f.uploaded, Some(Ok(_))));
                    let uploading = files.get().iter().any(|f| f.uploaded.is_none());
                    card.child(
                        input()
                            .element_ref(&field)
                            .value(move || text_sig.get())
                            .placeholder(placeholder.clone())
                            .placeholder_color(move || now(tokens::SUBTLE))
                            .selection_color(move || now(tokens::ACCENT))
                            .on_change(move |s: &str| text_sig.set(s.to_string()))
                            // Varias líneas: Enter agrega una línea; se manda con el botón.
                            .multiline(true)
                            .px(px(6.0))
                            // A la derecha, lugar para el ícono de quién contesta.
                            .pr(px(28.0))
                            .py(px(4.0))
                            .min_h(px(36.0))
                            .max_h(px(160.0))
                            .text_color(c(tokens::FG))
                            .text_size(px(15.0))
                            .font_family(theme::UI_FONT),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .child(round(38.0).border_1().border_color(c(tokens::CARD_BORDER)).active(|s| s.bg(c(tokens::ACTIVE))).on_press(move || menu.set(!menu.get())).child(theme::icon(theme::PLUS, tokens::MUTED, 20.0)))
                            .child(view().flex_1())
                            // Enviar; con el campo vacío y nada adjunto, grabar una nota de voz.
                            .child(dynamic(move || {
                                let empty = text_sig.get().trim().is_empty() && !ready;
                                let can = !empty && !uploading;
                                let (do_send, mic) = (do_send.clone(), mic.clone());
                                let voice = empty && crate::audio::supported();
                                round(38.0)
                                    .bg(c(if voice { tokens::HOVER } else if can { tokens::ACCENT } else { tokens::HOVER }))
                                    .active(|s| s.opacity(0.8))
                                    .on_press(move || if voice { mic() } else { do_send() })
                                    .child(if voice { theme::mic_icon(tokens::MUTED, 20.0).into_element() } else { theme::icon(theme::ARROW_UP, if can { tokens::ON_ACCENT } else { tokens::SUBTLE }, 20.0).into_element() })
                                    .into_element()
                            })),
                    )
                    .into_element()
                })),
        )
}
