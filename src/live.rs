//! Lo que un agente escribe mientras lo escribe: el texto aparece a un paso parejo (más rápido si se
//! atrasa) con un cursor al final, «pensando» o la herramienta que usa con un brillo que lo
//! recorre, y el botón para volver al último mensaje cuando se subió a leer.
//!
//! El hilo se vuelve a armar en cada sincronización, así que lo que tiene que durar (cuánto se
//! mostró, si se sigue el final) vive aquí y no en la vista.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

use nsh_core::ui::prelude::*;

use crate::theme::{c, tokens};

/// Caracteres por segundo cuando el texto va al día.
const PACE: f32 = 90.0;
/// Lo más que tarda en mostrarse lo atrasado, aunque llegue mucho de golpe.
const CATCH_UP: f32 = 0.35;
/// Una vuelta del brillo, en segundos.
const SHIMMER: f32 = 1.8;
/// Dónde sigue lo que falta.
pub const CARET: &str = "▍";

struct Reveal {
    shown: f32,
    at: Instant,
    text: String,
}

thread_local! {
    static REVEALS: RefCell<HashMap<String, Reveal>> = RefCell::new(HashMap::new());
    /// Algo en pantalla pidió otro cuadro (texto atrasado, un brillo, el cursor).
    static FRAME: Cell<bool> = const { Cell::new(false) };
    /// Se sigue el final del hilo (no se subió a leer) y dónde quedó si no.
    static FOLLOW: Cell<bool> = const { Cell::new(true) };
    static OFFSET: Cell<f32> = const { Cell::new(0.0) };
}

/// Cada cuadro de la animación; lo que se anima lo lee para volver a dibujarse.
#[derive(Default, Clone, Copy, PartialEq)]
pub struct Tick(pub u64);

/// El botón para bajar al último mensaje se ve.
#[derive(Default, Clone, Copy, PartialEq)]
pub struct ShowDown(pub bool);

/// Caracteres que se ven después de `elapsed`, de `shown` hacia `total`.
fn advanced(shown: f32, total: usize, elapsed: f32) -> f32 {
    let behind = total as f32 - shown;
    if behind <= 0.0 {
        return total as f32;
    }
    let rate = PACE.max(behind / CATCH_UP);
    (shown + rate * elapsed).min(total as f32)
}

/// Cuánto de `text` se ve ahora (de lo que se va escribiendo bajo `key`); pide otro cuadro si falta.
pub fn revealed(key: &str, text: &str) -> String {
    let total = text.chars().count();
    let now = Instant::now();
    let shown = REVEALS.with(|r| {
        let mut r = r.borrow_mut();
        let entry = r.entry(key.to_string()).or_insert(Reveal { shown: 0.0, at: now, text: String::new() });
        // Otro mensaje (el agente empezó a escribir de nuevo): desde el principio.
        let seen: String = entry.text.chars().take(entry.shown as usize).collect();
        if !text.starts_with(&seen) {
            entry.shown = 0.0;
        }
        entry.shown = advanced(entry.shown.min(total as f32), total, now.duration_since(entry.at).as_secs_f32());
        entry.at = now;
        entry.text = text.to_string();
        entry.shown as usize
    });
    if shown < total {
        FRAME.set(true);
    }
    text.chars().take(shown).collect()
}

/// Lo que ya terminó de escribirse no se sigue revelando.
pub fn forget(key: &str) {
    REVEALS.with(|r| r.borrow_mut().remove(key));
}

fn seconds() -> f32 {
    thread_local! { static START: Instant = Instant::now(); }
    START.with(|s| s.elapsed().as_secs_f32())
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |v: u32, s: u32| (v >> s) & 0xff;
    let m = |s: u32| ((ch(a, s) as f32) + (ch(b, s) as f32 - ch(a, s) as f32) * t).round() as u32;
    (m(16) << 16) | (m(8) << 8) | m(0)
}

/// Un texto con una franja de luz que lo recorre mientras espera (del tono apagado al del texto).
pub fn shimmer(label: &str, size: f32) -> Text {
    FRAME.set(true);
    let band = -0.3 + 1.6 * ((seconds() / SHIMMER).fract());
    let chars: Vec<char> = label.chars().collect();
    let count = chars.len().saturating_sub(1).max(1) as f32;
    let (dim, lit) = (tokens::SUBTLE, tokens::FG);
    let mut t = text("").text_size(px(size));
    for (ix, ch) in chars.iter().enumerate() {
        let x = ix as f32 / count;
        let k = (1.0 - (x - band).abs() / 0.25).max(0.0);
        t = t.child(text(ch.to_string()).text_color(c((mix(dim.0, lit.0, k), mix(dim.1, lit.1, k)))));
    }
    t
}

/// Si algo pidió otro cuadro desde el último (y lo olvida).
pub fn wants_frame() -> bool {
    FRAME.replace(false)
}

/// Al abrir un hilo se empieza siguiendo el final.
pub fn reset_follow() {
    FOLLOW.set(true);
    OFFSET.set(0.0);
    if global::<ShowDown>().get().0 {
        global::<ShowDown>().set(ShowDown(false));
    }
}

pub fn following() -> bool {
    FOLLOW.get()
}

pub fn offset() -> f32 {
    OFFSET.get()
}

/// Se desplazó la lista: sigue el final si quedó abajo del todo (con un margen).
pub fn scrolled(offset: f32, content: f32, view: f32) {
    let at_end = offset + view >= content - 48.0;
    FOLLOW.set(at_end);
    OFFSET.set(offset);
    if global::<ShowDown>().get().0 == at_end {
        global::<ShowDown>().set(ShowDown(!at_end));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn va_a_su_paso_y_se_pone_al_dia() {
        // Al día: 90 por segundo.
        assert_eq!(advanced(0.0, 1000, 0.1).round(), 9.0f32.max((1000.0 / CATCH_UP * 0.1f32).min(1000.0)).round());
        // Poco atrasado: el paso parejo.
        assert!((advanced(0.0, 20, 0.1) - 9.0).abs() < 0.01);
        // Nunca pasa del total.
        assert_eq!(advanced(10.0, 12, 5.0), 12.0);
    }

    #[test]
    fn otro_mensaje_empieza_de_nuevo() {
        let _ = revealed("k", "hola mundo");
        std::thread::sleep(std::time::Duration::from_millis(60));
        let a = revealed("k", "hola mundo");
        assert!(!a.is_empty());
        let b = revealed("k", "otra cosa");
        assert!(b.chars().count() <= 1, "{b}");
    }
}
