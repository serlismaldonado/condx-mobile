//! Entornos y trabajos de la Mac (solo lectura): lo que la Mac publica en `env:` y `jobs:`. Con el
//! mismo aspecto que la lista de hilos: una tarjeta con filas, el título arriba y una línea chica
//! con el estado (solo si pide atención) y los detalles.

use nsh_core::ui::prelude::*;

use crate::sync::{Env, Job};
use crate::theme::{self, c, kind_tag, muted, section, tokens, Health};

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

/// Los entornos agrupados por proyecto: un punto si su Convex responde (verde), no (rojo) o todavía
/// no se revisó (gris); el tipo (producción en rojo) y el deployment.
pub fn envs(list: Vec<Env>) -> View {
    if list.is_empty() {
        return empty("No environments yet", "The Mac publishes its environments here once it's linked to Condx Cloud.");
    }
    let mut col = view().gap_3();
    let mut projects: Vec<String> = list.iter().map(|e| e.project.clone()).collect();
    projects.dedup();
    for project in projects {
        let items: Vec<View> = list
            .iter()
            .filter(|e| e.project == project)
            .map(|e| {
                let health = match e.reachable {
                    Some(true) => Health::Ok,
                    Some(false) => Health::Bad,
                    None => Health::Unknown,
                };
                let state = match e.reachable {
                    Some(true) => "Responding",
                    Some(false) => "Not responding",
                    None => "Not checked yet",
                };
                view()
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
                    .child(
                        view()
                            .flex_row()
                            .gap(px(6.0))
                            .items_center()
                            .pl(px(16.0))
                            .child(text(state.to_string()).text_size(px(11.5)).text_color(c(if e.reachable == Some(false) { tokens::DANGER } else { tokens::SUBTLE })))
                            .child(if e.deployment.is_empty() {
                                view().into_element()
                            } else {
                                view().flex_row().gap(px(6.0)).flex_1().child(text("·").text_size(px(11.5)).text_color(c(tokens::SUBTLE))).child(view().flex_1().child(text(e.deployment.clone()).number_of_lines(1u32).text_size(px(11.5)).font_family(theme::MONO_FONT).text_color(c(tokens::SUBTLE)))).into_element()
                            }),
                    )
            })
            .collect();
        let title = if project.is_empty() { "heskala-erp".to_string() } else { project };
        col = col.child(view().gap_2().child(section(&title)).child(rows(items)));
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
                meta = meta.child(theme::dot(health)).child(text(word).text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(c(color))).child(text("·").text_size(px(11.5)).text_color(c(tokens::SUBTLE)));
            }
            let mut detail = vec![j.env.clone()];
            if !j.requested_by.is_empty() {
                detail.push(format!("by {}", j.requested_by));
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
                        .child(text(theme::ago(j.created_at).trim_end_matches(" ago").to_string()).text_size(px(11.5)).text_color(c(tokens::SUBTLE))),
                )
                .child(meta);
            if expanded {
                if let Some(e) = &j.error {
                    row = row.child(view().mt_1().px_2().py(px(6.0)).rounded(px(6.0)).bg(c(tokens::HOVER)).child(text(e.lines().take(8).collect::<Vec<_>>().join("\n")).number_of_lines(8u32).font_family(theme::MONO_FONT).text_size(px(11.5)).line_height(px(16.0)).text_color(c(tokens::DANGER))));
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
