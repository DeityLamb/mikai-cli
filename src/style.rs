//! Minimal ANSI styling for CLI output. Off when stdout is not a terminal,
//! under `NO_COLOR`, or with `--no-color`.

use std::{io::IsTerminal, sync::OnceLock};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

static COLOR: OnceLock<bool> = OnceLock::new();

pub fn init(disabled: bool) {
    let on = !disabled && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
    let _ = COLOR.set(on);
}

fn paint(code: &str, s: &str) -> String {
    if *COLOR.get().unwrap_or(&false) {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    paint("1", s)
}
pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn red(s: &str) -> String {
    paint("31", s)
}
pub fn green(s: &str) -> String {
    paint("32", s)
}
pub fn yellow(s: &str) -> String {
    paint("33", s)
}
pub fn magenta(s: &str) -> String {
    paint("35", s)
}
pub fn cyan(s: &str) -> String {
    paint("36", s)
}

/// Cuts `s` to `width` terminal columns, adding `…` when it had to cut.
pub fn truncate(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > width {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Pads (or cuts) `s` to exactly `width` columns.
pub fn pad(s: &str, width: usize) -> String {
    let s = truncate(s, width);
    let fill = width.saturating_sub(s.width());
    format!("{s}{}", " ".repeat(fill))
}
