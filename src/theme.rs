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
    view().flex_row().gap_2().items_center().child(dot(health)).child(text(label.into()).text_size(px(12.0)).text_color(c(tokens::MUTED)))
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
    view()
        .px(px(6.0))
        .h(px(18.0))
        .justify_center()
        .border_1()
        .border_color(if prod { c(tokens::PROD) } else { c(tokens::CARD_BORDER) })
        .child(text(label).text_size(px(11.0)).text_color(if prod { c(tokens::PROD) } else { c(tokens::MUTED) }))
}

/// Bloque con borde sutil y recto.
pub fn card() -> View {
    view().gap_2().p_3().border_1().border_color(c(tokens::CARD_BORDER)).bg(c(tokens::CARD))
}

/// Etiqueta de sección: versalitas pequeñas (como «ENVIRONMENTS» en escritorio).
pub fn section(title: &str) -> impl IntoElement {
    text(title.to_uppercase()).text_size(px(11.0)).text_color(c(tokens::SUBTLE))
}

pub fn muted(label: impl Into<String>) -> impl IntoElement {
    text(label.into()).text_size(px(12.5)).text_color(c(tokens::MUTED))
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
    let label: String = label.into();
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
        .placeholder(placeholder)
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
    let s = ((crate::sync::now_ms() - ms) / 1000).max(0);
    match s {
        0..=59 => format!("{s}s ago"),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

/// Deslizar para confirmar, como el «slide to unlock» del iPhone: la perilla amarilla sigue al
/// dedo (en nativo, sin trabajo por cuadro) y, si llega casi al final, `on_done`; si no, vuelve.
/// Reemplaza escribir el nombre del entorno: es igual de deliberado y no se hace por accidente.
pub fn slide_to_confirm(label: impl Into<String>, on_done: impl Fn() + Clone + 'static) -> impl IntoElement {
    let label: String = label.into();
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
