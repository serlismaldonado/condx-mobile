//! Entornos y trabajos de la Mac (solo lectura): lo que la Mac publica en `env:` y `jobs:`. Con el
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
                let state = crate::i18n::t(state);
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

