//! Markdown de los agentes en el teléfono, con el estilo de la app de escritorio (`answer_view`):
//! títulos, listas, citas, `código` en línea, **negrita**, *cursiva*, enlaces, tablas, y bloques de
//! código con su lenguaje y colores (palabras clave, textos, números, comentarios). Nada se desborda:
//! los bloques de código y las tablas anchas se desplazan de lado dentro de su caja y el resto se
//! ajusta al ancho.

use nsh_core::ui::prelude::*;

use crate::theme::{c, tokens, MONO_FONT};

/// Un bloque de la respuesta.
enum Block {
    Para(String),
    Heading(u8, String),
    Bullet(String, String),
    Quote(String),
    Code(String, String),
    /// Encabezado y filas.
    Table(Vec<String>, Vec<Vec<String>>),
    Rule,
}

/// Las celdas de una fila de tabla (`| a | b |`), sin los bordes; `\|` es una barra dentro de la celda.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.replace("\\|", "\u{1}").split('|').map(|c| c.trim().replace('\u{1}', "|")).collect()
}

/// La línea que separa el encabezado de una tabla: `|---|:---:|`.
fn is_separator(line: &str) -> bool {
    let cs = cells(line);
    line.contains('-') && !cs.is_empty() && cs.iter().all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

/// Una tabla de markdown, por si el bloque tiene que desplazarse de lado (el gesto de responder no va).
pub fn has_table(src: &str) -> bool {
    let lines: Vec<&str> = src.lines().collect();
    lines.windows(2).any(|w| w[0].trim_start().starts_with('|') && is_separator(w[1]))
}

fn blocks(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let flush = |para: &mut Vec<String>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(para.join(" ")));
            para.clear();
        }
    };
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        let t = line.trim_end();
        let lead = t.trim_start();
        if let Some(lang) = lead.strip_prefix("```") {
            flush(&mut para, &mut out);
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(l.to_string());
            }
            out.push(Block::Code(lang.trim().to_string(), code.join("\n")));
        } else if lead.starts_with('|') && lines.peek().is_some_and(|next| is_separator(next)) {
            flush(&mut para, &mut out);
            lines.next();
            let head = cells(lead);
            let mut rows = Vec::new();
            while let Some(row) = lines.peek().filter(|l| l.trim_start().starts_with('|')) {
                let mut r = cells(row);
                r.resize(head.len(), String::new());
                rows.push(r);
                lines.next();
            }
            out.push(Block::Table(head, rows));
        } else if lead.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(h) = lead.strip_prefix("### ").or_else(|| lead.strip_prefix("#### ")) {
            flush(&mut para, &mut out);
            out.push(Block::Heading(3, h.to_string()));
        } else if let Some(h) = lead.strip_prefix("## ") {
            flush(&mut para, &mut out);
            out.push(Block::Heading(2, h.to_string()));
        } else if let Some(h) = lead.strip_prefix("# ") {
            flush(&mut para, &mut out);
            out.push(Block::Heading(1, h.to_string()));
        } else if matches!(lead, "---" | "***" | "___") {
            flush(&mut para, &mut out);
            out.push(Block::Rule);
        } else if let Some(q) = lead.strip_prefix("> ").or_else(|| (lead == ">").then_some("")) {
            flush(&mut para, &mut out);
            match out.last_mut() {
                Some(Block::Quote(prev)) => {
                    prev.push(' ');
                    prev.push_str(q);
                }
                _ => out.push(Block::Quote(q.to_string())),
            }
        } else if let Some(item) = lead.strip_prefix("- ").or_else(|| lead.strip_prefix("* ")).or_else(|| lead.strip_prefix("• ")) {
            flush(&mut para, &mut out);
            let indent = if t.len() - lead.len() >= 2 { "◦" } else { "•" };
            out.push(Block::Bullet(indent.into(), item.to_string()));
        } else if let Some((n, item)) = lead.split_once(". ").filter(|(n, _)| !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit())) {
            flush(&mut para, &mut out);
            out.push(Block::Bullet(format!("{n}."), item.to_string()));
        } else {
            para.push(lead.to_string());
        }
    }
    flush(&mut para, &mut out);
    out
}

/// Un tramo con estilo dentro de un párrafo.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Style {
    Plain,
    Bold,
    Italic,
    Code,
    /// Código dentro de negrita (`**`next`**`, como escribe Claude).
    BoldCode,
    Link,
}

/// `código`, **negrita**, *cursiva* / _cursiva_ y [enlace](url) (se muestra el texto).
fn spans(src: &str) -> Vec<(Style, String)> {
    let mut out: Vec<(Style, String)> = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let push = |out: &mut Vec<(Style, String)>, plain: &mut String| {
        if !plain.is_empty() {
            out.push((Style::Plain, std::mem::take(plain)));
        }
    };
    let find = |from: usize, pat: &[char]| -> Option<usize> { (from..chars.len().saturating_sub(pat.len() - 1)).find(|&j| chars[j..j + pat.len()] == *pat) };
    while i < chars.len() {
        let ch = chars[i];
        if ch == '`'
            && let Some(end) = find(i + 1, &['`']) {
                push(&mut out, &mut plain);
                out.push((Style::Code, chars[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        if ch == '*' && chars.get(i + 1) == Some(&'*')
            && let Some(end) = find(i + 2, &['*', '*']).filter(|&e| e > i + 2) {
                push(&mut out, &mut plain);
                // Lo de adentro también tiene tramos: el código sigue siendo código (en negrita).
                let inner: String = chars[i + 2..end].iter().collect();
                for (style, s) in spans(&inner) {
                    out.push((match style {
                        Style::Code => Style::BoldCode,
                        Style::Plain => Style::Bold,
                        other => other,
                    }, s));
                }
                i = end + 2;
                continue;
            }
        if (ch == '*' || ch == '_') && chars.get(i + 1).is_some_and(|c| !c.is_whitespace()) && (i == 0 || !chars[i - 1].is_alphanumeric())
            && let Some(end) = find(i + 1, &[ch]).filter(|&e| e > i + 1 && !chars[e - 1].is_whitespace()) {
                push(&mut out, &mut plain);
                out.push((Style::Italic, chars[i + 1..end].iter().collect()));
                i = end + 1;
                continue;
            }
        if ch == '['
            && let Some(close) = find(i + 1, &[']'])
                && chars.get(close + 1) == Some(&'(')
                    && let Some(end) = find(close + 2, &[')']) {
                        push(&mut out, &mut plain);
                        out.push((Style::Link, chars[i + 1..close].iter().collect()));
                        i = end + 1;
                        continue;
                    }
        plain.push(ch);
        i += 1;
    }
    push(&mut out, &mut plain);
    out
}

/// Un párrafo con sus tramos.
pub fn inline(src: &str, size: f32, color: (u32, u32)) -> Text {
    let mut t = text("").text_size(px(size)).line_height(px(size * 1.45)).text_color(c(color));
    for (style, s) in spans(src) {
        t = t.child(match style {
            Style::Plain => text(s),
            Style::Bold => text(s).font_weight(FontWeight::SEMIBOLD),
            Style::Italic => text(s).italic(),
            Style::Code => text(s).font_family(MONO_FONT).text_size(px(size - 1.5)).text_color(c(tokens::CODE_INLINE)),
            Style::BoldCode => text(s).font_family(MONO_FONT).font_weight(FontWeight::SEMIBOLD).text_size(px(size - 1.5)).text_color(c(tokens::CODE_INLINE)),
            Style::Link => text(s).underline().text_color(c(tokens::INFO)),
        });
    }
    t
}

const KEYWORDS: &[&str] = &[
    "const", "let", "var", "function", "return", "if", "else", "for", "while", "import", "export", "from", "async", "await", "new", "class", "interface", "type", "extends", "throw", "try", "catch",
    "fn", "pub", "use", "mut", "impl", "struct", "enum", "match", "self", "Self", "def", "None", "True", "False", "true", "false", "null", "undefined", "in", "of", "as", "default",
];

/// Una línea de código en tramos de color.
fn highlight(line: &str) -> Text {
    let mut t = text("");
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut word = String::new();
    let flush_word = |t: Text, word: &mut String| -> Text {
        if word.is_empty() {
            return t;
        }
        let w = std::mem::take(word);
        let tone = if KEYWORDS.contains(&w.as_str()) {
            Some(tokens::SYN_KEYWORD)
        } else if w.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            Some(tokens::SYN_NUMBER)
        } else {
            None
        };
        match tone {
            Some(tone) => t.child(text(w).text_color(c(tone))),
            None => t.child(text(w)),
        }
    };
    while i < chars.len() {
        let ch = chars[i];
        // Comentarios hasta el final de la línea.
        if (ch == '/' && chars.get(i + 1) == Some(&'/')) || (ch == '#' && (i == 0 || chars[i - 1] == ' ')) {
            t = flush_word(t, &mut word);
            return t.child(text(chars[i..].iter().collect::<String>()).italic().text_color(c(tokens::SUBTLE)));
        }
        if ch == '"' || ch == '\'' || ch == '`' {
            t = flush_word(t, &mut word);
            let end = (i + 1..chars.len()).find(|&j| chars[j] == ch && chars[j - 1] != '\\').unwrap_or(chars.len() - 1);
            t = t.child(text(chars[i..=end].iter().collect::<String>()).text_color(c(tokens::SYN_STRING)));
            i = end + 1;
            continue;
        }
        if ch.is_alphanumeric() || ch == '_' || ch == '.' && word.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            word.push(ch);
        } else {
            t = flush_word(t, &mut word);
            t = t.child(text(ch.to_string()));
        }
        i += 1;
    }
    flush_word(t, &mut word)
}

/// El código abierto a pantalla completa (lenguaje, código); `None` cerrado. Lo dibuja la app encima
/// de todo (`fullscreen`).
#[derive(Default, Clone)]
pub struct OpenCode(pub Option<(String, String)>);

/// Las líneas del código en tramos de color, sin cortarlas (la caja se desplaza).
fn code_lines(code: &str, numbers: bool) -> View {
    let mut lines = view();
    let width = code.lines().count().to_string().len();
    for (ix, l) in code.lines().enumerate() {
        let line = highlight(if l.is_empty() { " " } else { l }).font_family(MONO_FONT).text_size(px(12.0)).line_height(px(18.0)).text_color(c(tokens::FG));
        lines = lines.child(if numbers {
            view().flex_row().gap_3().child(text(format!("{:>width$}", ix + 1)).font_family(MONO_FONT).text_size(px(12.0)).line_height(px(18.0)).text_color(c(tokens::SUBTLE))).child(line).into_element()
        } else {
            line.into_element()
        });
    }
    lines
}

fn header_button(label: &'static str, on_press: impl Fn() + 'static) -> View {
    view().px_2().h(px(26.0)).justify_center().rounded(px(5.0)).active(|s| s.bg(c(tokens::ACTIVE))).on_press(on_press).child(text(label).text_size(px(12.0)).text_color(c(tokens::MUTED)))
}

/// Un bloque de código: arriba el lenguaje, copiar y abrirlo a pantalla completa; las líneas
/// enteras en una caja que se desplaza de lado.
pub fn code_block(lang: &str, code: &str) -> View {
    let (copy, open) = (code.to_string(), (lang.to_string(), code.to_string()));
    view()
        .rounded(px(8.0))
        .overflow_hidden()
        .border_1()
        .border_color(c(tokens::CARD_BORDER))
        .bg(c(tokens::CODE_BG))
        .child(
            view()
                .flex_row()
                .items_center()
                .pl_3()
                .pr_1()
                .h(px(30.0))
                .border_b_1()
                .border_color(c(tokens::CARD_BORDER))
                .child(view().flex_1().child(text(if lang.is_empty() { "code".to_string() } else { lang.to_string() }).text_size(px(11.0)).letter_spacing(px(0.4)).text_color(c(tokens::SUBTLE))))
                .child(header_button("Copy", move || set_clipboard(copy.clone())))
                .child(header_button("Expand", move || global::<OpenCode>().set(OpenCode(Some(open.clone()))))),
        )
        .child(scroll().horizontal().child(view().px_3().py_2().child(code_lines(code, false))))
}

/// El código a pantalla completa: números de línea, desplazamiento en las dos direcciones, copiar y cerrar.
pub fn fullscreen() -> impl IntoElement {
    dynamic(move || {
        let Some((lang, code)) = global::<OpenCode>().get().0.clone() else {
            return view().into_element();
        };
        let copy = code.clone();
        view()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .z_index(10)
            .bg(c(tokens::CODE_BG))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .pl_4()
                    .pr_2()
                    .h(px(48.0))
                    .border_b_1()
                    .border_color(c(tokens::CARD_BORDER))
                    .bg(c(tokens::GROUND))
                    .child(view().flex_1().child(text(if lang.is_empty() { "Code".to_string() } else { lang.clone() }).text_size(px(14.0)).font_weight(FontWeight::MEDIUM).text_color(c(tokens::FG))))
                    .child(header_button("Copy", move || set_clipboard(copy.clone())))
                    .child(header_button("Close", move || global::<OpenCode>().set(OpenCode(None)))),
            )
            .child(scroll().flex_1().child(scroll().horizontal().child(view().p_4().child(code_lines(&code, true)))))
            .into_element()
    })
}

/// La respuesta entera.
/// Una tabla: columnas de ancho fijo según lo que llevan (el texto se ajusta dentro de la celda), el
/// encabezado en negrita sobre un fondo, y de lado se desplaza si no entra.
fn table(head: &[String], rows: &[Vec<String>], size: f32) -> View {
    let plain = |s: &str| s.replace("**", "").replace('`', "").chars().count();
    let widths: Vec<f32> = (0..head.len())
        .map(|i| {
            let longest = std::iter::once(&head[i]).chain(rows.iter().map(|r| &r[i])).map(|c| plain(c)).max().unwrap_or(0);
            (longest as f32 * size * 0.56 + 20.0).clamp(64.0, 230.0)
        })
        .collect();
    // Más ancha que la pantalla del teléfono: una tarjeta por fila (el primer valor de título, el resto
    // como «columna: valor»), que se lee sin desplazarse de lado.
    if widths.iter().sum::<f32>() > 330.0 && head.len() > 2 {
        let mut col = view().gap(px(6.0));
        for r in rows {
            let mut card = view().gap(px(3.0)).px(px(10.0)).py(px(8.0)).rounded(px(8.0)).border_1().border_color(c(tokens::CARD_BORDER));
            card = card.child(inline(r.first().map(String::as_str).unwrap_or(""), size, tokens::FG).font_weight(FontWeight::SEMIBOLD));
            for (h, v) in head.iter().zip(r.iter()).skip(1).filter(|(_, v)| !v.trim().is_empty()) {
                let line = format!("{}: {v}", h.replace("**", ""));
                card = card.child(inline(&line, size - 1.0, tokens::MUTED));
            }
            col = col.child(card);
        }
        return col;
    }
    let row = |cells: &[String], header: bool, last: bool| {
        let mut r = view().flex_row();
        if header {
            r = r.bg(c(tokens::PANEL));
        }
        if !last {
            r = r.border_b_1().border_color(c(tokens::CARD_BORDER));
        }
        for (i, cell) in cells.iter().enumerate() {
            let t = inline(cell, size - 1.0, tokens::FG);
            r = r.child(view().w(px(widths[i])).px(px(10.0)).py(px(7.0)).child(if header { t.font_weight(FontWeight::SEMIBOLD) } else { t }));
        }
        r
    };
    let mut grid = view().child(row(head, true, rows.is_empty()));
    for (ix, r) in rows.iter().enumerate() {
        grid = grid.child(row(r, false, ix + 1 == rows.len()));
    }
    view()
        .rounded(px(8.0))
        .overflow_hidden()
        .border_1()
        .border_color(c(tokens::CARD_BORDER))
        .child(scroll().horizontal().keyboard_taps(KeyboardTaps::Handled).child(grid))
}

pub fn render(src: &str, size: f32) -> View {
    let mut col = view().gap(px(8.0));
    for b in blocks(src) {
        col = col.child(match b {
            Block::Para(p) => inline(&p, size, tokens::FG).into_element(),
            Block::Heading(level, h) => inline(&h, if level == 1 { size + 2.0 } else { size + 0.5 }, tokens::FG).font_weight(FontWeight::SEMIBOLD).into_element(),
            Block::Bullet(mark, item) => view()
                .flex_row()
                .gap_2()
                .child(view().w(px(16.0)).flex_shrink_0().child(text(mark).text_size(px(size)).line_height(px(size * 1.45)).text_color(c(tokens::MUTED))))
                .child(view().flex_1().child(inline(&item, size, tokens::FG)))
                .into_element(),
            Block::Quote(q) => view().flex_row().child(view().w(px(3.0)).rounded(px(2.0)).bg(c(tokens::CARD_BORDER))).child(view().flex_1().pl_3().child(inline(&q, size, tokens::MUTED))).into_element(),
            Block::Code(lang, code) => code_block(&lang, &code).into_element(),
            Block::Table(head, rows) => table(&head, &rows, size).into_element(),
            Block::Rule => view().h(px(1.0)).bg(c(tokens::CARD_BORDER)).into_element(),
        });
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separa_bloques_y_tramos() {
        let b = blocks("Looked at it.\n\n```ts\nconst a = 1;\n```\n- one\n2. two\n## Cause\n> note");
        assert!(matches!(&b[0], Block::Para(p) if p == "Looked at it."));
        assert!(matches!(&b[1], Block::Code(l, c) if l == "ts" && c == "const a = 1;"));
        assert!(matches!(&b[2], Block::Bullet(m, i) if m == "•" && i == "one"));
        assert!(matches!(&b[3], Block::Bullet(m, _) if m == "2."));
        assert!(matches!(&b[4], Block::Heading(2, h) if h == "Cause"));
        assert!(matches!(&b[5], Block::Quote(q) if q == "note"));
        let s = spans("reads `card.last4` **now** and *maybe* [docs](https://x)");
        let kinds: Vec<_> = s.iter().filter(|(k, _)| *k != Style::Plain).map(|(k, t)| (*k, t.as_str())).collect();
        assert!(kinds == vec![(Style::Code, "card.last4"), (Style::Bold, "now"), (Style::Italic, "maybe"), (Style::Link, "docs")]);
        let t = blocks("Hoy:\n\n| ID | Causa |\n|---|:---:|\n| `j1` | a \\| b |\n| j2 |\nfin");
        assert!(matches!(&t[1], Block::Table(h, r) if h == &["ID", "Causa"] && r.len() == 2 && r[0][1] == "a | b" && r[1] == ["j2", ""]));
        assert!(matches!(&t[2], Block::Para(p) if p == "fin"));
        assert!(has_table("| a |\n|---|") && !has_table("a | b\nno"));
        // Sin cierre: queda como texto.
        assert_eq!(spans("a `b"), vec![(Style::Plain, "a `b".to_string())]);
    }

    #[test]
    fn codigo_dentro_de_negrita() {
        let s = spans("**`next`** 16.1.4 y **muy `importante`**");
        assert_eq!(s[0], (Style::BoldCode, "next".into()));
        assert_eq!(s[1], (Style::Plain, " 16.1.4 y ".into()));
        assert_eq!(s[2], (Style::Bold, "muy ".into()));
        assert_eq!(s[3], (Style::BoldCode, "importante".into()));
    }
}
