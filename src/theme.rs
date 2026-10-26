//! El tema de la app de escritorio de Condx (`crates/app/src/theme.rs` y `ui.rs`), en el teléfono:
//! plano y compacto, al estilo de Zed: grises neutros, bordes sutiles y rectos, sin sombras, IBM
//! Plex Sans y JetBrains Mono (en `assets/fonts`), y el amarillo de la marca solo en lo puntual
//! (el botón principal). Cada tono lleva su valor claro y oscuro: sigue al sistema, como la app de
//! escritorio con el tema «sistema».

use nsh_core::color::rgb;
use nsh_core::ui::prelude::*;
use nsh_core::ui::AdaptiveColor;

pub const UI_FONT: &str = "IBM Plex Sans";
pub const MONO_FONT: &str = "JetBrains Mono";

/// Un tono con su valor en claro y en oscuro.
const fn tone(light: u32, dark: u32) -> (u32, u32) {
    (light, dark)
}

fn adaptive((light, dark): (u32, u32)) -> AdaptiveColor {
    AdaptiveColor { light: rgb(light).into(), dark: rgb(dark).into() }
}

/// Los tonos de la app de escritorio (`Tokens::of`).
pub mod tokens {
    use super::tone;
    /// Barra superior (la lateral en escritorio).
    pub const GROUND: (u32, u32) = tone(0xf6f6f6, 0x181818);
    /// El contenido.
    pub const PANEL: (u32, u32) = tone(0xffffff, 0x1c1c1c);
    pub const CARD: (u32, u32) = tone(0xffffff, 0x1c1c1c);
    pub const CARD_BORDER: (u32, u32) = tone(0xe4e4e4, 0x2b2b2b);
    pub const HOVER: (u32, u32) = tone(0xefefef, 0x262626);
    pub const ACTIVE: (u32, u32) = tone(0xe7e7e7, 0x2d2d2d);
    pub const FG: (u32, u32) = tone(0x1f1f1f, 0xd6d6d6);
    pub const MUTED: (u32, u32) = tone(0x666666, 0x8f8f8f);
    pub const SUBTLE: (u32, u32) = tone(0x8c8c8c, 0x6b6b6b);
    /// Amarillo de la marca (#F3B01C), para rellenos.
    pub const ACCENT: (u32, u32) = tone(0xf3b01c, 0xf3b01c);
    /// Texto sobre el amarillo (9,5:1).
    pub const ON_ACCENT: (u32, u32) = tone(0x1f1503, 0x1f1503);
    /// Estados (los de GPUI Kit en la app de escritorio).
    pub const SUCCESS: (u32, u32) = tone(0x16a34a, 0x22c55e);
    pub const WARNING: (u32, u32) = tone(0xd97706, 0xf59e0b);
    pub const DANGER: (u32, u32) = tone(0xdc2626, 0xef4444);
    /// Código: el fondo de los bloques, el código en línea y los colores del resaltado.
    pub const CODE_BG: (u32, u32) = tone(0xf6f6f4, 0x151515);
    pub const CODE_INLINE: (u32, u32) = tone(0xb4233c, 0xf08ca0);
    pub const SYN_KEYWORD: (u32, u32) = tone(0x7c3aed, 0xc4a7ff);
    pub const SYN_STRING: (u32, u32) = tone(0x15803d, 0x86d9a0);
    pub const SYN_NUMBER: (u32, u32) = tone(0xb45309, 0xf5b44a);
    /// Lo que escribe la persona en un hilo: el amarillo de la marca, apenas (sutil, no compite).
    /// Lo que flota sobre el contenido (el campo de escribir).
    pub const RAISED: (u32, u32) = tone(0xffffff, 0x262626);
    pub const USER_BUBBLE: (u32, u32) = tone(0xfdf4dc, 0x342d1c);
    pub const USER_BUBBLE_BORDER: (u32, u32) = tone(0xf0dda8, 0x4d4124);
    /// Para distinguir agentes (la inicial en el chat).
    pub const INFO: (u32, u32) = tone(0x2563eb, 0x60a5fa);
    pub const VIOLET: (u32, u32) = tone(0x7c3aed, 0xa78bfa);
    /// Producción (`kind_tag`).
    pub const PROD: (u32, u32) = tone(0xd9534b, 0xe5756e);
}

pub fn c(t: (u32, u32)) -> AdaptiveColor {
    adaptive(t)
}

/// Un tono para lo que solo toma un color (el del placeholder de un campo).
pub fn now(t: (u32, u32)) -> Hsla {
    nsh_core::ui::ambient::color(adaptive(t))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Health {
    Ok,
    Warn,
    Bad,
    Unknown,
}

pub fn dot(health: Health) -> impl IntoElement {
    let tone = match health {
        Health::Ok => tokens::SUCCESS,
        Health::Warn => tokens::WARNING,
        Health::Bad => tokens::DANGER,
        Health::Unknown => tokens::SUBTLE,
    };
    view().w(px(7.0)).h(px(7.0)).rounded(px(4.0)).bg(c(tone))
}

/// Estado como punto de color y texto apagado.
pub fn status_tag(health: Health, label: impl Into<String>) -> impl IntoElement {
    // El texto se corta a dos líneas (un error largo no se sale de la pantalla).
    view().flex_row().gap_2().items_center().child(dot(health)).child(view().flex_1().child(text(crate::i18n::t(&label.into())).number_of_lines(2u32).text_size(px(12.0)).text_color(c(tokens::MUTED))))
}

/// El estado de algo en una píldora de su color (punto, texto y borde).
pub fn pill(health: Health, label: impl Into<String>) -> impl IntoElement {
    let tone = match health {
        Health::Ok => tokens::SUCCESS,
        Health::Warn => tokens::WARNING,
        Health::Bad => tokens::DANGER,
        Health::Unknown => tokens::SUBTLE,
    };
    view().flex_row().gap(px(5.0)).items_center().px(px(7.0)).h(px(20.0)).rounded(px(10.0)).border_1().border_color(c(tone)).child(dot(health)).child(text(crate::i18n::t(&label.into())).text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(c(tone)))
}

/// Tipo de entorno: texto pequeño con borde; solo producción lleva color.
pub fn kind_tag(kind: &str) -> impl IntoElement {
    let prod = kind == "cloud-prod";
    let label = match kind {
        "cloud-prod" => "production",
        "cloud-dev" => "cloud dev",
        "devbox" => "devbox",
        "throwaway" => "throwaway",
        "" => "—",
        other => other,
    }
    .to_string();
    let label = crate::i18n::t(&label);
    // Producción con su marco rojo; los demás, solo el texto (sin borde: no compite con el estado).
    let tag = view().h(px(18.0)).justify_center().child(text(label).text_size(px(11.0)).text_color(if prod { c(tokens::PROD) } else { c(tokens::SUBTLE) }));
    if prod { tag.px(px(6.0)).border_1().border_color(c(tokens::PROD)) } else { tag }
}

/// Bloque con borde sutil y recto.
pub fn card() -> View {
    view().gap_2().p_3().border_1().border_color(c(tokens::CARD_BORDER)).bg(c(tokens::CARD))
}

/// Etiqueta de sección: versalitas pequeñas (como «ENVIRONMENTS» en escritorio).
pub fn section(title: &str) -> impl IntoElement {
    text(crate::i18n::t(title).to_uppercase()).text_size(px(11.0)).text_color(c(tokens::SUBTLE))
}

pub fn muted(label: impl Into<String>) -> impl IntoElement {
    text(crate::i18n::t(&label.into())).text_size(px(12.5)).text_color(c(tokens::MUTED))
}

pub fn mono(label: impl Into<String>, size: f32) -> impl IntoElement {
    text(label.into()).font_family(MONO_FONT).text_size(px(size))
}

/// Fila clave/valor.
pub fn kv(label: &str, value: impl IntoElement) -> impl IntoElement {
    view().flex_row().gap_3().items_center().child(view().w(px(84.0)).child(muted(label.to_string()))).child(view().flex_1().child(value))
}

#[derive(Clone, Copy, PartialEq)]
pub enum Variant {
    /// El amarillo de la marca con texto oscuro.
    Primary,
    /// Con contorno.
    Outline,
    /// Contorno y texto rojos (rechazar).
    Danger,
}

pub fn button(label: impl Into<String>, variant: Variant, on_press: impl Fn() + 'static) -> impl IntoElement {
    let label: String = crate::i18n::t(&label.into());
    let (bg, border, fg) = match variant {
        Variant::Primary => (Some(tokens::ACCENT), tokens::ACCENT, tokens::ON_ACCENT),
        Variant::Outline => (None, tokens::CARD_BORDER, tokens::FG),
        Variant::Danger => (None, tokens::DANGER, tokens::DANGER),
    };
    let mut b = view().flex_1().h(px(40.0)).items_center().justify_center().border_1().border_color(c(border));
    if let Some(bg) = bg {
        b = b.bg(c(bg));
    }
    b.active(move |s| if variant == Variant::Primary { s.opacity(0.85) } else { s.bg(c(tokens::ACTIVE)) })
        .on_press(on_press)
        .child(text(label).text_size(px(13.5)).font_weight(FontWeight::MEDIUM).text_color(c(fg)))
}

/// Campo de texto plano: borde sutil, sin esquinas redondeadas.
pub fn field(placeholder: &'static str, value: Signal<String>) -> Input {
    input()
        .value(move || value.get())
        .placeholder(crate::i18n::t(placeholder))
        .placeholder_color(move || now(tokens::SUBTLE))
        .selection_color(move || now(tokens::ACCENT))
        .auto_correct(false)
        .on_change(move |t: &str| value.set(t.to_string()))
        .px_3()
        .h(px(40.0))
        .border_1()
        .border_color(c(tokens::CARD_BORDER))
        .bg(c(tokens::CARD))
        .text_color(c(tokens::FG))
        .text_size(px(14.0))
        .font_family(UI_FONT)
}

/// «12s ago», «3 min ago», «2 h ago», «4 d ago» (como `ui::ago`).
pub fn ago(ms: i64) -> String {
    let span = short(ms);
    if crate::i18n::spanish() { format!("hace {span}") } else { format!("{span} ago") }
}

/// Cuánto hace, sin «hace»: «3 min».
pub fn short(ms: i64) -> String {
    let s = ((crate::sync::now_ms() - ms) / 1000).max(0);
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min", s / 60),
        3600..=86_399 => format!("{} h", s / 3600),
        _ => format!("{} d", s / 86_400),
    }
}

/// Deslizar para confirmar, como el «slide to unlock» del iPhone: la perilla amarilla sigue al
/// dedo (en nativo, sin trabajo por cuadro) y, si llega casi al final, `on_done`; si no, vuelve.
/// Reemplaza escribir el nombre del entorno: es igual de deliberado y no se hace por accidente.
pub fn slide_to_confirm(label: impl Into<String>, on_done: impl Fn() + Clone + 'static) -> impl IntoElement {
    let label: String = crate::i18n::t(&label.into());
    const KNOB: f32 = 48.0;
    const HEIGHT: f32 = 48.0;
    let width = signal(0.0f32);
    view()
        .h(px(HEIGHT))
        .border_1()
        .border_color(c(tokens::CARD_BORDER))
        .bg(c(tokens::HOVER))
        .on_layout(move |b: &Bounds<Points>| {
            let w = f32::from(b.size.width);
            if (w - width.get()).abs() > 0.5 {
                width.set(w);
            }
        })
        .child(dynamic(move || {
            let on_done = on_done.clone();
            // Hasta dónde llega la perilla (el ancho del carril menos ella y los bordes).
            let max = (width.get() - KNOB - 2.0).max(1.0);
            let x = animated_value(0.0);
            let knob_x = x.interpolate(&[0.0, max], &[0.0, max]);
            let label_fade = x.interpolate(&[0.0, max * 0.6], &[1.0, 0.0]);
            view()
                .flex_1()
                .child(
                    view()
                        .absolute()
                        .left(px(0.0))
                        .right(px(0.0))
                        .top(px(0.0))
                        .bottom(px(0.0))
                        .items_center()
                        .justify_center()
                        .bind_opacity(&label_fade)
                        .child(text(label.clone()).text_size(px(13.5)).text_color(c(tokens::MUTED))),
                )
                .child(
                    view()
                        .absolute()
                        .left(px(0.0))
                        .top(px(0.0))
                        .w(px(KNOB))
                        .h(px(HEIGHT - 2.0))
                        .items_center()
                        .justify_center()
                        .bg(c(tokens::ACCENT))
                        .bind_translate_x(&knob_x)
                        .claim_touches(TouchClaim::on_pan(PanActivation::horizontal(px(6.0), px(12.0))))
                        .pan_values(Some(&x), None)
                        .on_pan_end(move |e: &PanEvent| {
                            let at = e.value_x.unwrap_or(0.0);
                            if at >= max * 0.9 {
                                let _ = x.spring(max, Spring::default());
                                on_done();
                            } else {
                                let _ = x.spring(0.0, Spring::default());
                            }
                        })
                        .child(text("→").text_size(px(18.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::ON_ACCENT))),
                )
                .into_element()
        }))
}

/// Botón chico (30 de alto, ancho justo): las acciones de una tarjeta, que no compiten con su
/// contenido.
pub fn small_button(label: impl Into<String>, variant: Variant, on_press: impl Fn() + 'static) -> View {
    let label: String = crate::i18n::t(&label.into());
    let (bg, border, fg) = match variant {
        Variant::Primary => (Some(tokens::ACCENT), tokens::ACCENT, tokens::ON_ACCENT),
        Variant::Outline => (None, tokens::CARD_BORDER, tokens::FG),
        Variant::Danger => (None, tokens::DANGER, tokens::DANGER),
    };
    let mut b = view().h(px(30.0)).px_3().items_center().justify_center().rounded(px(6.0)).border_1().border_color(c(border));
    if let Some(bg) = bg {
        b = b.bg(c(bg));
    }
    b.active(move |s| if variant == Variant::Primary { s.opacity(0.85) } else { s.bg(c(tokens::ACTIVE)) })
        .on_press(on_press)
        .child(text(label).text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(c(fg)))
}

/// Control segmentado (las pestañas): un carril gris con la elegida en blanco, y un número cuando
/// algo espera.
/// Una pestaña: nombre, cuántas cosas esperan, si es la elegida y qué hace al tocarla.
pub type Segment = (&'static str, usize, bool, Box<dyn Fn()>);

pub fn segmented(items: Vec<Segment>) -> View {
    let mut rail = view().flex_row().h(px(34.0)).p(px(3.0)).gap(px(3.0)).rounded(px(8.0)).bg(c(tokens::HOVER));
    for (label, count, selected, on_press) in items {
        let mut seg = view()
            .flex_1()
            .flex_row()
            .gap(px(6.0))
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .on_press(on_press)
            .child(text(crate::i18n::t(label)).text_size(px(13.0)).font_weight(if selected { FontWeight::MEDIUM } else { FontWeight::NORMAL }).text_color(c(if selected { tokens::FG } else { tokens::MUTED })));
        if selected {
            seg = seg.bg(c(tokens::CARD)).border_1().border_color(c(tokens::CARD_BORDER));
        }
        if count > 0 {
            seg = seg.child(
                view().min_w(px(18.0)).h(px(18.0)).px(px(5.0)).rounded(px(9.0)).items_center().justify_center().bg(c(tokens::ACCENT)).child(text(count.to_string()).text_size(px(11.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::ON_ACCENT))),
            );
        }
        rail = rail.child(seg);
    }
    rail
}

/// Un ícono de trazo fino (los de Lucide, como los de Claude) dibujado en un canvas desde sus trazos
/// SVG (24×24): nsh 0.2 no trae íconos. Sigue al tema claro u oscuro.
pub fn icon(paths: &'static [&'static str], color: (u32, u32), size: f32) -> CanvasView {
    let k = size / 24.0;
    canvas(move || {
        let mut p = Painter::new().scale(k, k).stroke_style(now(color)).line_width(1.8).line_cap(nsh_core::canvas::LineCap::Round).line_join(nsh_core::canvas::LineJoin::Round);
        for d in paths {
            p = p.stroke_svg(d);
        }
        p
    })
    .w(px(size))
    .h(px(size))
}

pub const MIC: &[&str] = &["M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z", "M19 10v2a7 7 0 0 1-14 0v-2", "M12 19v3"];
/// Enviar.
pub const ARROW_UP: &[&str] = &["m5 12 7-7 7 7", "M12 19V5"];
pub const ARROW_DOWN: &[&str] = &["M12 5v14", "m19 12-7 7-7-7"];
/// La marca de Condx (assets/brand/mark.svg del repo condx).
pub const CONDX_MARK: &[&str] = &["M18 5.5A9 9 0 1 0 18 18.5", "M15 9a4.5 4.5 0 1 0 0 6"];
/// Agregar (un hilo nuevo).
/// Rama de git (Lucide «git-branch»).
pub const BRANCH: &[&str] = &["M6 3v12", "M18 3a3 3 0 1 0 0 6a3 3 0 1 0 0-6Z", "M6 15a3 3 0 1 0 0 6a3 3 0 1 0 0-6Z", "M18 9a9 9 0 0 1-9 9"];

pub const PLUS: &[&str] = &["M5 12h14", "M12 5v14"];
/// Una laptop (la Mac contesta) y una nube (contesta el sitio en la nube). Trazos de Lucide (ISC).
/// Las pestañas de abajo (trazos de Lucide, ISC).
pub const TAB_THREADS: &[&str] = &["M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"];
pub const TAB_APPROVALS: &[&str] = &["M22 11.08V12a10 10 0 1 1-5.93-9.14", "m9 11 3 3L22 4"];
pub const TAB_JOBS: &[&str] = &["M8 6h13", "M8 12h13", "M8 18h13", "M3 6h.01", "M3 12h.01", "M3 18h.01"];
pub const TAB_ENVS: &[&str] = &["M4 2h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2Z", "M4 14h16a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2v-4a2 2 0 0 1 2-2Z", "M6 6h.01", "M6 18h.01"];
pub const TAB_INCIDENTS: &[&str] = &["m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3", "M12 9v4", "M12 17h.01"];
pub const SLIDERS: &[&str] = &["M20 7h-9", "M14 17H5", "M17 14a3 3 0 1 0 0 6a3 3 0 1 0 0-6Z", "M7 4a3 3 0 1 0 0 6a3 3 0 1 0 0-6Z"];
pub const LAPTOP: &[&str] = &["M20 16V7a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v9", "M2 20h20", "M4 16h16l1.5 4h-19Z"];
pub const CLOUD: &[&str] = &["M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"];
/// Escribir un mensaje nuevo (lápiz sobre un cuadrado).
pub const COMPOSE: &[&str] = &["M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7", "M18.375 2.625a1 1 0 0 1 3 3l-9.013 9.014a2 2 0 0 1-.853.505l-2.873.84a.5.5 0 0 1-.62-.62l.84-2.873a2 2 0 0 1 .506-.852z"];

pub fn mic_icon(color: (u32, u32), size: f32) -> CanvasView {
    icon(MIC, color, size)
}

/// El tema elegido en Ajustes (`system`, `light`, `dark`), guardado en el teléfono.
pub fn saved_scheme() -> String {
    crate::secure::data_dir()
        .and_then(|d| std::fs::read_to_string(d.join("prefs.json")).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["theme"].as_str().map(str::to_string))
        .unwrap_or_else(|| "system".into())
}

/// Aplica el tema (sistema, claro u oscuro) y lo guarda.
pub fn apply_scheme(scheme: &str, save: bool) {
    let appearance = match scheme {
        "light" => nsh_core::ui::Appearance::Light,
        "dark" => nsh_core::ui::Appearance::Dark,
        _ => nsh_core::ui::Appearance::System,
    };
    nsh_core::ui::ambient::set_color_scheme(appearance);
    if save && let Some(d) = crate::secure::data_dir() {
        let _ = std::fs::write(d.join("prefs.json"), serde_json::json!({ "theme": scheme }).to_string());
    }
}

/// Una pestaña de la barra de abajo: ícono y nombre corto, con lo que espera.
pub type Tab = (&'static str, &'static str, &'static [&'static str], usize);

/// La barra de pestañas de abajo (como en las apps del teléfono): la elegida en el color del texto,
/// las demás apagadas; lo que espera, en un globo ámbar sobre el ícono.
pub fn tab_bar(tabs: Vec<Tab>, current: &str, on_pick: impl Fn(&'static str) + Clone + 'static) -> View {
    let mut bar = view().flex_row().h(px(56.0)).border_t_1().border_color(c(tokens::CARD_BORDER)).bg(c(tokens::GROUND));
    for (key, label, paths, count) in tabs {
        let on = key == current;
        let color = if on { tokens::FG } else { tokens::SUBTLE };
        let pick = on_pick.clone();
        let mut glyph = view().child(icon_view(paths, color, 21.0));
        if count > 0 {
            glyph = glyph.child(
                view()
                    .absolute()
                    .top(px(-4.0))
                    .right(px(-9.0))
                    .min_w(px(16.0))
                    .h(px(16.0))
                    .px(px(4.0))
                    .rounded(px(8.0))
                    .items_center()
                    .justify_center()
                    .bg(c(tokens::ACCENT))
                    .child(text(count.to_string()).text_size(px(10.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::ON_ACCENT))),
            );
        }
        bar = bar.child(
            view()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(3.0))
                .active(|s| s.opacity(0.6))
                .on_press(move || pick(key))
                .child(glyph)
                .child(text(crate::i18n::t(label)).text_size(px(10.5)).font_weight(if on { FontWeight::MEDIUM } else { FontWeight::NORMAL }).text_color(c(color))),
        );
    }
    bar
}

fn icon_view(paths: &'static [&'static str], color: (u32, u32), size: f32) -> View {
    view().child(icon(paths, color, size))
}

