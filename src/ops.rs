//! Entornos (con el resumen y los servicios locales) y trabajos de la Mac: lo que la Mac publica en `env:` y `jobs:`. Con el
//! mismo aspecto que la lista de hilos: una tarjeta con filas, el título arriba y una línea chica
//! con el estado (solo si pide atención) y los detalles.

use nsh_core::ui::prelude::*;

use std::rc::Rc;

use serde_json::{json, Value};

use crate::sync::{Env, FlowInfo, Job, RecipeInfo, RunState};
use crate::theme::{self, c, kind_tag, muted, section, status_tag, tokens, Health, Variant};

/// La tarjeta con sus filas separadas por una línea.
fn rows(items: Vec<View>) -> View {
    let mut col = view().border_1().border_color(c(tokens::CARD_BORDER)).rounded(px(10.0)).overflow_hidden();
    let n = items.len();
    for (ix, row) in items.into_iter().enumerate() {
        let row = row.px_3().py(px(12.0)).bg(c(tokens::CARD));
        col = col.child(if ix + 1 < n { row.border_b_1().border_color(c(tokens::CARD_BORDER)) } else { row });
    }
    col
}

fn empty(title: &'static str, detail: &'static str) -> View {
    view()
        .items_center()
        .gap_2()
        .py(px(48.0))
        .px_4()
        .child(text(title).text_size(px(15.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG)))
        .child(text(detail).text_size(px(13.0)).line_height(px(19.0)).text_center().text_color(c(tokens::MUTED)))
}

/// Lo que la pestaña de entornos necesita además de los entornos: el resumen de la Mac, los números
/// de arriba y cómo pedir algo a un servicio.
pub struct EnvsCtx {
    pub overview: Value,
    /// El resumen del sitio en la nube: sus servicios corren ahí cuando la Mac está apagada.
    pub site: Value,
    /// Si el sitio en la nube contesta (la Mac está apagada y el sitio, encendido).
    pub cloud_on: bool,
    pub running_jobs: usize,
    pub approvals: usize,
    /// Si la Mac está encendida: los servicios corren en ella, sin ella no se manejan.
    pub mac_on: bool,
    pub state: RunState,
    /// Si este teléfono pidió algo a un servicio (para mostrar en qué quedó).
    pub asked: Signal<bool>,
    pub send: Rc<dyn Fn(Value)>,
    /// Los proyectos abiertos (los demás, plegados en una línea con su estado).
    pub open: Signal<Vec<String>>,
}

/// Un número con su nombre, como las tarjetas de arriba de la vista general de la Mac.
fn stat(value: String, label: &str, tone: (u32, u32)) -> View {
    view()
        .flex_1()
        .gap(px(2.0))
        .px(px(10.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(c(tokens::CARD_BORDER))
        .bg(c(tokens::CARD))
        .child(text(value).text_size(px(18.0)).font_weight(FontWeight::MEDIUM).text_color(c(tone)))
        .child(text(crate::i18n::t(label)).number_of_lines(1u32).text_size(px(10.5)).text_color(c(tokens::MUTED)))
}

/// Una línea chica de detalles separados por «·» (vacíos fuera).
fn details(parts: Vec<String>, tone: (u32, u32), mono: bool) -> View {
    let line = parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · ");
    let t = text(line).number_of_lines(1u32).text_size(px(11.5)).text_color(c(tone));
    view().flex_row().pl(px(16.0)).child(view().flex_1().child(if mono { t.font_family(theme::MONO_FONT) } else { t }))
}

fn megabytes(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
    } else {
        format!("{} MB", bytes >> 20)
    }
}

/// Un servicio local (backend, deployer, frontend): estado, uso y sus botones. `branch`: la rama de
/// la Mac, para que el sitio en la nube siga desde ahí.
fn service_row(project: &str, svc: &Value, ctx: &EnvsCtx, branch: &str) -> View {
    let st = |k: &str| svc[k].as_str().unwrap_or_default().to_string();
    let status = st("status");
    let busy = svc["busy"].as_str().map(str::to_string);
    let error = svc["error"].as_str().filter(|e| !e.is_empty()).map(str::to_string);
    let healthy = svc["healthy"].as_bool();
    let (health, word) = match (&busy, status.as_str()) {
        (Some(b), _) => (Health::Warn, match b.as_str() { "start" => "Starting…", "stop" => "Stopping…", _ => "Restarting…" }),
        (None, "running") if healthy == Some(false) => (Health::Warn, "Not responding"),
        (None, "running") => (Health::Ok, "Running"),
        (None, "external") => (Health::Warn, "Port in use"),
        _ if error.is_some() => (Health::Bad, "Failed"),
        _ => (Health::Unknown, "Stopped"),
    };
    let mut parts = vec![crate::i18n::t(word)];
    if let Some(cpu) = svc["cpu"].as_f64() {
        parts.push(format!("{cpu:.0}% CPU"));
    }
    if let Some(mem) = svc["memory"].as_u64() {
        parts.push(megabytes(mem));
    }
    // La fase del deployer cuenta mientras corre (o si quedó en error).
    if let Some(phase) = svc["phase"].as_str().filter(|p| !p.is_empty() && *p != "unknown" && (status == "running" || *p == "error")) {
        parts.push(crate::i18n::t(phase));
    }
    let mut title_row = view()
        .flex_row()
        .gap_2()
        .items_center()
        .child(theme::dot(health))
        .child(view().flex_1().child(text(st("title")).number_of_lines(1u32).text_size(px(14.5)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))));
    let can_ask = ctx.mac_on || ctx.cloud_on;
    if let Some(url) = svc["openUrl"].as_str().filter(|_| status == "running").map(str::to_string) {
        title_row = title_row.child(theme::small_button("Open", Variant::Outline, move || {
            let _ = crate::scan::open_url(&url);
        }));
    }
    if can_ask && busy.is_none() {
        let ask = |action: &'static str| {
            let send = ctx.send.clone();
            let asked = ctx.asked;
            let (project, name, branch) = (project.to_string(), st("name"), branch.to_string());
            move || {
                asked.set(true);
                send(json!({ "service": { "project": project, "name": name, "action": action, "branch": branch } }));
            }
        };
        title_row = if status == "running" {
            title_row.child(theme::small_button("Restart", Variant::Outline, ask("restart"))).child(theme::small_button("Stop", Variant::Outline, ask("stop")))
        } else if status == "external" {
            title_row
        } else {
            title_row.child(theme::small_button("Start", Variant::Outline, ask("start")))
        };
    }
    let mut row = view().gap(px(4.0)).child(title_row).child(details(parts, if health == Health::Bad { tokens::DANGER } else { tokens::SUBTLE }, false));
    if let Some(e) = error {
        row = row.child(view().pl(px(16.0)).child(text(e).number_of_lines(3u32).text_size(px(11.5)).line_height(px(16.0)).text_color(c(tokens::DANGER))));
    } else if status == "external" {
        row = row.child(details(vec![st("detail")], tokens::SUBTLE, false));
    }
    row
}

/// Un entorno: si su Convex responde, cuánto tarda, su versión y qué commit tiene.
fn env_row(e: &Env, extra: &Value) -> View {
    let health = match e.reachable {
        Some(true) => Health::Ok,
        Some(false) => Health::Bad,
        None => Health::Unknown,
    };
    let state = crate::i18n::t(match e.reachable {
        Some(true) => "Responding",
        Some(false) => "Not responding",
        None => "Not checked yet",
    });
    let latency = extra["latencyMs"].as_i64().filter(|_| e.reachable == Some(true)).map(|ms| format!("{ms} ms")).unwrap_or_default();
    let mut row = view()
        .gap(px(4.0))
        .child(
            view()
                .flex_row()
                .gap_2()
                .items_center()
                .child(theme::dot(health))
                .child(view().flex_1().child(text(e.name.clone()).number_of_lines(1u32).text_size(px(14.5)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))))
                .child(kind_tag(&e.kind)),
        )
        .child(details(vec![state, latency], if e.reachable == Some(false) { tokens::DANGER } else { tokens::SUBTLE }, false));
    let mut code = vec![e.deployment.clone()];
    if let Some(commit) = extra["commit"].as_str() {
        code.push(commit.to_string());
    }
    if let Some(n) = extra["behind"].as_i64().filter(|n| *n > 0) {
        let branch = extra["branch"].as_str().unwrap_or("main");
        code.push(crate::i18n::tr(format!("{n} behind {branch}"), format!("{n} detrás de {branch}")));
    }
    if code.iter().any(|p| !p.is_empty()) {
        row = row.child(details(code, tokens::SUBTLE, true));
    }
    row
}

/// La pestaña de entornos, como la vista general de la Mac: arriba los números (entornos en línea,
/// trabajos que corren, por aprobar, lo que pide atención); después cada proyecto con lo que pide
/// atención, su repositorio, sus entornos y sus servicios locales (levantar, detener, reiniciar).
pub fn envs(list: Vec<Env>, ctx: EnvsCtx) -> View {
    if list.is_empty() {
        return empty("No environments yet", "The Mac publishes its environments here once it's linked to Condx Cloud.");
    }
    let summaries: Vec<Value> = ctx.overview["projects"].as_array().cloned().unwrap_or_default();
    let summary_of = |name: &str| summaries.iter().find(|p| p["name"] == name).cloned().unwrap_or(Value::Null);
    // El backend local apagado no es una alerta (se levanta cuando se usa): solo cuentan los demás.
    let watched: Vec<&Env> = list.iter().filter(|e| e.kind != "devbox").collect();
    let online = watched.iter().filter(|e| e.reachable == Some(true)).count();
    let attention: usize = summaries.iter().map(|p| p["attention"].as_array().map(Vec::len).unwrap_or(0)).sum();
    let mut col = view().gap_3().child(
        view()
            .flex_row()
            .gap_2()
            .child(stat(format!("{online}/{}", watched.len()), "Online", if online < watched.len() { tokens::DANGER } else { tokens::FG }))
            .child(stat(ctx.running_jobs.to_string(), "Running", tokens::FG))
            .child(stat(ctx.approvals.to_string(), "To approve", if ctx.approvals > 0 { tokens::WARNING } else { tokens::FG }))
            .child(stat(attention.to_string(), "Attention", if attention > 0 { tokens::DANGER } else { tokens::FG })),
    );
    // En qué quedó lo último que se pidió a un servicio.
    if ctx.asked.get() && (ctx.state.waiting || ctx.state.result.is_some()) {
        let (health, line) = match &ctx.state.result {
            None => (Health::Unknown, crate::i18n::t("Sending your request to the Mac…")),
            Some(r) if r.starts_with("rejected") => (Health::Bad, r.trim_start_matches("rejected: ").to_string()),
            Some(r) => (Health::Ok, r.clone()),
        };
        col = col.child(status_tag(health, line));
    }
    if !ctx.mac_on && ctx.cloud_on {
        col = col.child(muted("The Mac is off: local services run on the cloud site, with their own data."));
    } else if !ctx.mac_on {
        col = col.child(muted("The Mac is off: its local services can be started when it's back."));
    }
    let mut projects: Vec<String> = list.iter().map(|e| e.project.clone()).collect();
    projects.dedup();
    for project in projects {
        let sum = summary_of(&project);
        let expanded = ctx.open.get().contains(&project);
        // Plegado: el nombre con su estado en una línea (en línea, avisos, servicios que corren).
        let mine: Vec<&Env> = list.iter().filter(|e| e.project == project).collect();
        let down = mine.iter().filter(|e| e.reachable == Some(false) && e.kind != "devbox").count();
        let notes_n = sum["attention"].as_array().map(Vec::len).unwrap_or(0);
        let up = sum["services"].as_array().into_iter().flatten().filter(|s| s["status"] == "running").count();
        let n = mine.len();
        let mut glance = vec![crate::i18n::tr(format!("{n} env{}", if n == 1 { "" } else { "s" }), format!("{n} entorno{}", if n == 1 { "" } else { "s" }))];
        if down > 0 {
            glance.push(crate::i18n::tr(format!("{down} down"), format!("{down} caído{}", if down == 1 { "" } else { "s" })));
        }
        if notes_n > 0 {
            glance.push(crate::i18n::tr(format!("{notes_n} warning{}", if notes_n == 1 { "" } else { "s" }), format!("{notes_n} aviso{}", if notes_n == 1 { "" } else { "s" })));
        }
        if up > 0 {
            glance.push(crate::i18n::tr(format!("{up} running"), format!("{up} corriendo")));
        }
        let health = if down > 0 { Health::Bad } else if notes_n > 0 { Health::Warn } else { Health::Ok };
        let toggle = {
            let open = ctx.open;
            let p = project.clone();
            move || {
                let mut v = open.get();
                if let Some(i) = v.iter().position(|x| *x == p) {
                    v.remove(i);
                } else {
                    v.push(p.clone());
                }
                theme::set_pref("openProjects", json!(v));
                open.set(v);
            }
        };
        let header = view()
            .flex_row()
            .gap_2()
            .items_center()
            .py(px(6.0))
            .active(|s| s.opacity(0.6))
            .on_press(toggle)
            .child(theme::icon(if expanded { theme::CHEVRON_DOWN } else { theme::CHEVRON_RIGHT }, tokens::SUBTLE, 14.0))
            .child(section(&project))
            .child(view().flex_1())
            .child(theme::dot(health))
            .child(text(glance.join(" · ")).number_of_lines(1u32).text_size(px(11.5)).text_color(c(tokens::SUBTLE)));
        if !expanded {
            col = col.child(header);
            continue;
        }
        let mut block = view().gap_2().child(header);
        // Lo que pide atención, como en la Mac: en ámbar, una línea por cosa.
        let notes: Vec<View> = sum["attention"].as_array().into_iter().flatten().filter_map(|a| a.as_str()).map(|a| status_tag(Health::Warn, a.to_string()).into_element()).map(|e| view().child(e)).collect();
        if !notes.is_empty() {
            block = block.child(rows(notes));
        }
        // Entornos y, si la Mac lo publicó, el repositorio en la misma tarjeta.
        let mut items: Vec<View> = list.iter().filter(|e| e.project == project).map(|e| env_row(e, &sum["envs"][&e.name])).collect();
        let git = &sum["git"];
        if git["branch"].is_string() {
            let ahead = git["ahead"].as_i64().unwrap_or(0);
            let changes = git["changed"].as_i64().unwrap_or(0) + git["untracked"].as_i64().unwrap_or(0);
            let mut parts = vec![];
            if ahead > 0 {
                parts.push(crate::i18n::tr(format!("{ahead} unpushed"), format!("{ahead} sin subir")));
            }
            if changes > 0 {
                parts.push(crate::i18n::tr(format!("{changes} changes"), format!("{changes} cambios")));
            }
            if parts.is_empty() {
                parts.push(crate::i18n::t("Clean"));
            }
            items.push(
                view()
                    .gap(px(4.0))
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .child(theme::icon(theme::BRANCH, tokens::SUBTLE, 13.0))
                            .child(view().flex_1().child(text(format!("{} · {}", git["branch"].as_str().unwrap_or(""), git["head"].as_str().unwrap_or(""))).number_of_lines(1u32).text_size(px(13.5)).font_family(theme::MONO_FONT).text_color(c(tokens::FG))))
                            .child(text(parts.join(" · ")).text_size(px(11.5)).text_color(c(if ahead > 0 || changes > 0 { tokens::WARNING } else { tokens::SUBTLE }))),
                    )
                    .child(details(vec![git["subject"].as_str().unwrap_or("").to_string()], tokens::SUBTLE, false)),
            );
        }
        block = block.child(rows(items));
        // Los servicios locales: el backend, el deployer y el frontend del proyecto.
        // Con la Mac apagada, los del sitio en la nube (el mismo proyecto, clonado allá, contra su cloud dev).
        let site_p = ctx.site["projects"].as_array().into_iter().flatten().find(|p| p["name"] == project.as_str()).cloned().unwrap_or(Value::Null);
        let services: Vec<Value> = if ctx.mac_on { sum["services"].as_array().cloned().unwrap_or_default() } else { site_p["services"].as_array().cloned().unwrap_or_default() };
        let mac_branch = git["branch"].as_str().unwrap_or("").to_string();
        // Seguir en la nube: la rama de la Mac tiene que estar en GitHub. Con la Mac encendida, se le
        // pide que la suba; con la Mac apagada, se dice qué rama tiene la nube y si falta algo.
        let ahead = git["ahead"].as_i64().unwrap_or(0);
        let changes = git["changed"].as_i64().unwrap_or(0) + git["untracked"].as_i64().unwrap_or(0);
        let main_branch = matches!(mac_branch.as_str(), "main" | "master" | "dev" | "develop" | "");
        if ctx.mac_on && ahead > 0 && !main_branch {
            let send = ctx.send.clone();
            let asked = ctx.asked;
            let p = project.clone();
            block = block.child(
                view()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(view().flex_1().child(muted(crate::i18n::tr(format!("{ahead} commits only on the Mac: push them to continue in the cloud"), format!("{ahead} commits solo en la Mac: súbelos para seguir en la nube")))))
                    .child(theme::small_button("Push branch", Variant::Outline, move || {
                        asked.set(true);
                        send(json!({ "handoff": { "project": p } }));
                    })),
            );
        } else if !ctx.mac_on && site_p.is_object() {
            let cloud_branch = site_p["git"]["branch"].as_str().unwrap_or("").to_string();
            let mut parts = vec![crate::i18n::tr(format!("Cloud: {cloud_branch}"), format!("Nube: {cloud_branch}"))];
            if !mac_branch.is_empty() && mac_branch != cloud_branch {
                parts.push(crate::i18n::tr(format!("Mac: {mac_branch} (switches when you start)"), format!("Mac: {mac_branch} (cambia al levantar)")));
            }
            if ahead > 0 || changes > 0 {
                parts.push(crate::i18n::tr(format!("{} only on the Mac", ahead + changes), format!("{} solo en la Mac", ahead + changes)));
            }
            block = block.child(details(parts, if ahead + changes > 0 { tokens::WARNING } else { tokens::SUBTLE }, false));
        }
        if !services.is_empty() {
            let stopped = services.iter().filter(|s| s["status"] != "running").count();
            let where_ = if ctx.mac_on {
                crate::i18n::t("Local services")
            } else {
                match site_p["target"]["env"].as_str() {
                    Some(env) => crate::i18n::tr(format!("On the cloud site → {env}"), format!("En la nube → {env}")),
                    None => crate::i18n::t("On the cloud site (no cloud dev key yet)"),
                }
            };
            let mut head = view().flex_row().items_center().pt(px(4.0)).child(view().flex_1().child(muted(where_)));
            if (ctx.mac_on || ctx.cloud_on) && services.iter().all(|s| s["busy"].is_null()) {
                let (label, action) = if stopped > 0 { ("Start all", "start") } else { ("Stop all", "stop") };
                let send = ctx.send.clone();
                let asked = ctx.asked;
                let p = project.clone();
                let b = mac_branch.clone();
                head = head.child(theme::small_button(label, Variant::Outline, move || {
                    asked.set(true);
                    send(json!({ "service": { "project": p, "name": "all", "action": action, "branch": b } }));
                }));
            }
            block = block.child(head).child(rows(services.iter().map(|s| service_row(&project, s, &ctx, &mac_branch)).collect()));
        }
        col = col.child(block);
    }
    col
}

/// Cuánto duró un trabajo (desde que empezó a correr: sin la espera de la aprobación).
fn took(job: &Job) -> Option<String> {
    let end = job.ended_at?;
    let s = ((end - job.started_at.unwrap_or(job.created_at)) / 1000).max(0);
    Some(match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    })
}

/// El estado de un trabajo: el punto y la palabra solo si corre, espera o salió mal.
fn job_state(status: &str) -> Option<(Health, &'static str)> {
    Some(match status {
        "running" => (Health::Warn, "Running"),
        "queued" => (Health::Warn, "Queued"),
        "waiting-approval" => (Health::Warn, "Needs approval"),
        "failed" => (Health::Bad, "Failed"),
        "expired" => (Health::Unknown, "Expired"),
        "rejected" => (Health::Unknown, "Rejected"),
        "cancelled" => (Health::Unknown, "Cancelled"),
        _ => return None,
    })
}

/// Los trabajos recientes: título y cuándo; abajo el estado (si pide atención), el entorno, quién lo
/// pidió y cuánto tardó. Tocarlo muestra el error y el commit.
pub fn jobs(list: Vec<Job>, open: Signal<Option<String>>) -> View {
    if list.is_empty() {
        return empty("No jobs yet", "Deploys, functions, backups and releases the Mac runs show up here.");
    }
    let items: Vec<View> = list
        .into_iter()
        .map(|j| {
            let id = j.id.clone();
            let expanded = open.get().as_deref() == Some(j.id.as_str());
            let prod = j.env_kind == "cloud-prod";
            let mut meta = view().flex_row().gap(px(6.0)).items_center();
            if let Some((health, word)) = job_state(&j.status) {
                let color = match health {
                    Health::Bad => tokens::DANGER,
                    Health::Warn => tokens::WARNING,
                    _ => tokens::MUTED,
                };
                meta = meta.child(theme::dot(health)).child(text(crate::i18n::t(word)).text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(c(color))).child(text("·").text_size(px(11.5)).text_color(c(tokens::SUBTLE)));
            }
            let mut detail = vec![j.env.clone()];
            if !j.requested_by.is_empty() {
                detail.push(crate::i18n::tr(format!("by {}", j.requested_by), format!("por {}", j.requested_by)));
            }
            if let Some(t) = took(&j) {
                detail.push(t);
            }
            meta = meta.child(view().flex_1().child(text(detail.join(" · ")).number_of_lines(1u32).text_size(px(11.5)).text_color(c(if prod { tokens::DANGER } else { tokens::SUBTLE }))));
            let mut row = view()
                .gap(px(5.0))
                .active(|s| s.bg(c(tokens::ACTIVE)))
                .on_press(move || open.set(if expanded { None } else { Some(id.clone()) }))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .child(view().flex_1().child(text(j.title.clone()).number_of_lines(1u32).text_size(px(14.5)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))))
                        .child(text(theme::short(j.created_at)).text_size(px(11.5)).text_color(c(tokens::SUBTLE))),
                )
                .child(meta);
            if expanded {
                if let Some(e) = &j.error {
                    row = row.child(view().mt_1().px_2().py(px(6.0)).rounded(px(6.0)).bg(c(tokens::HOVER)).child(text(e.lines().take(8).collect::<Vec<_>>().join("\n")).number_of_lines(8u32).font_family(theme::MONO_FONT).text_size(px(11.5)).line_height(px(16.0)).text_color(c(tokens::DANGER))));
                }
                // La cola del registro (la Mac la publica de lo que corre o falló).
                if let Some(log) = &j.log {
                    let tail: Vec<&str> = log.lines().collect();
                    let tail = tail[tail.len().saturating_sub(30)..].join("\n");
                    row = row.child(view().mt_1().px_2().py(px(6.0)).rounded(px(6.0)).bg(c(tokens::HOVER)).child(text(tail).font_family(theme::MONO_FONT).text_size(px(10.5)).line_height(px(15.0)).text_color(c(tokens::MUTED))));
                }
                let mut facts = vec![j.recipe.clone()];
                if let Some(commit) = &j.commit {
                    facts.push(commit.chars().take(8).collect());
                }
                row = row.child(view().mt_1().child(muted(facts.join(" · "))));
            }
            row
        })
        .collect();
    rows(items)
}

/// Lo que se está armando para pedir correr: qué (receta o flujo), dónde y con qué argumentos.
#[derive(Clone, Copy)]
pub struct RunForm {
    pub mode: Signal<Option<&'static str>>,
    pub env: Signal<Option<String>>,
    pub recipe: Signal<Option<String>>,
    pub args: Signal<Vec<String>>,
    pub flow: Signal<Option<String>>,
}

impl RunForm {
    pub fn new() -> Self {
        Self { mode: signal(None), env: signal(None), recipe: signal(None), args: signal(vec![]), flow: signal(None) }
    }

    fn reset(&self) {
        self.mode.set(None);
        self.env.set(None);
        self.recipe.set(None);
        self.args.set(vec![]);
        self.flow.set(None);
    }
}

/// Un chip para elegir (el elegido resaltado; producción en rojo).
fn chip(label: String, on: bool, danger: bool, pick: impl Fn() + 'static) -> View {
    let (border, fg) = if on { (tokens::ACCENT, tokens::FG) } else if danger { (tokens::PROD, tokens::PROD) } else { (tokens::CARD_BORDER, tokens::MUTED) };
    view()
        .px_3()
        .h(px(30.0))
        .justify_center()
        .rounded(px(15.0))
        .border_1()
        .border_color(c(border))
        .bg(c(if on { tokens::HOVER } else { tokens::CARD }))
        .active(|s| s.opacity(0.7))
        .on_press(pick)
        .child(text(label).text_size(px(12.5)).font_weight(if on { FontWeight::MEDIUM } else { FontWeight::NORMAL }).text_color(c(fg)))
}

/// Pedir correr una receta o un flujo desde el teléfono (la Mac lo verifica y aplica las mismas
/// aprobaciones que allá) y cómo va lo último pedido.
pub fn run_panel(form: RunForm, envs: Vec<Env>, recipes: Vec<RecipeInfo>, flows: Vec<FlowInfo>, state: RunState, send: Rc<dyn Fn(Value)>) -> View {
    let mut col = view().gap_3();
    let mode = form.mode.get();
    col = col.child(
        view()
            .flex_row()
            .gap_2()
            .child(theme::small_button("Run a job", if mode == Some("job") { Variant::Primary } else { Variant::Outline }, move || form.mode.set(if form.mode.get() == Some("job") { None } else { Some("job") })))
            .child(theme::small_button("Run a flow", if mode == Some("flow") { Variant::Primary } else { Variant::Outline }, move || form.mode.set(if form.mode.get() == Some("flow") { None } else { Some("flow") }))),
    );
    if state.waiting {
        col = col.child(status_tag(Health::Unknown, "Sending your request to the Mac…"));
    } else if let Some(r) = state.result.clone() {
        col = col.child(status_tag(if r.starts_with("rejected") { Health::Bad } else { Health::Ok }, r));
    }
    match mode {
        Some("job") => {
            let mut card = theme::card().rounded(px(10.0)).gap_3();
            let mut chips = view().flex_row().flex_wrap().gap_2();
            for e in &envs {
                let name = e.name.clone();
                let on = form.env.get().as_deref() == Some(name.as_str());
                chips = chips.child(chip(name.clone(), on, e.kind == "cloud-prod", move || {
                    form.env.set(Some(name.clone()));
                    form.recipe.set(None);
                }));
            }
            card = card.child(muted("Environment")).child(chips);
            if let Some(env) = envs.iter().find(|e| form.env.get().as_deref() == Some(e.name.as_str())) {
                let fits: Vec<&RecipeInfo> = recipes.iter().filter(|r| r.kinds.iter().any(|k| *k == env.kind)).collect();
                let mut chips = view().flex_row().flex_wrap().gap_2();
                for r in &fits {
                    let (name, defaults) = (r.name.clone(), r.args.iter().map(|a| a.2.clone().unwrap_or_default()).collect::<Vec<_>>());
                    let on = form.recipe.get().as_deref() == Some(name.as_str());
                    chips = chips.child(chip(r.title.clone(), on, false, move || {
                        form.recipe.set(Some(name.clone()));
                        form.args.set(defaults.clone());
                    }));
                }
                card = card.child(muted("What to run")).child(if fits.is_empty() { muted("Nothing can run on this environment from here.").into_element() } else { chips.into_element() });
                if let Some(r) = fits.iter().find(|r| form.recipe.get().as_deref() == Some(r.name.as_str())) {
                    for (ix, (name, kind, _)) in r.args.iter().enumerate() {
                        let value = form.args;
                        card = card.child(
                            view().gap_1().child(muted(format!("{name} · {kind}"))).child(
                                input()
                                    .value(move || value.get().get(ix).cloned().unwrap_or_default())
                                    .placeholder_color(move || theme::now(tokens::SUBTLE))
                                    .auto_correct(false)
                                    .on_change(move |t: &str| {
                                        let mut v = value.get();
                                        if v.len() <= ix {
                                            v.resize(ix + 1, String::new());
                                        }
                                        v[ix] = t.to_string();
                                        value.set(v);
                                    })
                                    .px_3()
                                    .h(px(38.0))
                                    .border_1()
                                    .border_color(c(tokens::CARD_BORDER))
                                    .rounded(px(8.0))
                                    .text_size(px(13.5))
                                    .text_color(c(tokens::FG)),
                            ),
                        );
                    }
                    let (recipe, env_name, names) = (r.name.clone(), env.name.clone(), r.args.iter().map(|a| a.0.clone()).collect::<Vec<_>>());
                    let send = send.clone();
                    let prod = env.kind == "cloud-prod";
                    card = card.child(theme::slide_to_confirm(crate::i18n::tr(format!("Slide to run “{}” on {}", r.title, env.name), format!("Desliza para correr «{}» en {}", r.title, env.name)), move || {
                        let values = form.args.get();
                        let args: serde_json::Map<String, Value> = names.iter().enumerate().filter_map(|(ix, n)| values.get(ix).filter(|v| !v.is_empty()).map(|v| (n.clone(), json!(v)))).collect();
                        send(json!({ "recipe": recipe, "env": env_name, "args": args }));
                        form.reset();
                    }));
                    if prod {
                        card = card.child(muted("Production: the Mac asks for Touch ID before it runs."));
                    }
                }
            }
            col = col.child(card.child(view().flex_row().justify_end().child(theme::small_button("Cancel", Variant::Outline, move || form.reset()))));
        }
        Some("flow") => {
            let mut card = theme::card().rounded(px(10.0)).gap_3();
            if flows.is_empty() {
                card = card.child(muted("No flows turned on. Create or turn one on in Condx on the Mac (Workflows)."));
            } else {
                let mut chips = view().flex_row().flex_wrap().gap_2();
                for f in &flows {
                    let id = f.id.clone();
                    let on = form.flow.get().as_deref() == Some(id.as_str());
                    chips = chips.child(chip(format!("{} · {}", f.name, f.project), on, false, move || form.flow.set(Some(id.clone()))));
                }
                card = card.child(muted("Flow")).child(chips);
                if let Some(f) = flows.iter().find(|f| form.flow.get().as_deref() == Some(f.id.as_str())) {
                    let (id, send) = (f.id.clone(), send.clone());
                    card = card.child(theme::slide_to_confirm(crate::i18n::tr(format!("Slide to run “{}”", f.name), format!("Desliza para correr «{}»", f.name)), move || {
                        send(json!({ "flow": id }));
                        form.reset();
                    }));
                }
            }
            col = col.child(card.child(view().flex_row().justify_end().child(theme::small_button("Cancel", Variant::Outline, move || form.reset()))));
        }
        _ => {}
    }
    col
}

