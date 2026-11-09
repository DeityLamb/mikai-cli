//! The TUI's design system. Every screen builds from these pieces so that
//! surfaces, keys, headers and lists look and behave the same everywhere.
//!
//! - Three surfaces: the terminal background, [`SURFACE`] for panels and
//!   blocks, [`OVERLAY`] for anything floating. [`SELECT`] marks the chosen row.
//! - Accent pink means "interactive": keys, focus, selection. Green means
//!   watched/done, yellow opened/in progress, red an error. Everything else is
//!   plain, bold or muted.
//! - Keys always look like [`key`] chips. Focus is a pink bar on the left.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::style::{pad, truncate};

pub const ACCENT: Color = Color::Rgb(224, 69, 123);
pub const SURFACE: Color = Color::Rgb(27, 26, 33);
/// Second shade for alternating transcript blocks.
pub const SURFACE_ALT: Color = Color::Rgb(34, 32, 42);
pub const OVERLAY: Color = Color::Rgb(40, 38, 50);
pub const SELECT: Color = Color::Rgb(80, 38, 62);
pub const KEY_BG: Color = Color::Rgb(56, 52, 68);
pub const MUTED: Color = Color::Rgb(130, 127, 143);
pub const FAINT: Color = Color::Rgb(78, 75, 90);
pub const GREEN: Color = Color::Rgb(112, 200, 140);
pub const YELLOW: Color = Color::Rgb(232, 192, 96);
pub const RED: Color = Color::Rgb(236, 104, 104);

pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

/// ` k ` key chip.
pub fn key(k: &str) -> Span<'static> {
    Span::styled(format!(" {k} "), Style::new().bg(KEY_BG).fg(ACCENT).add_modifier(Modifier::BOLD))
}

/// Key chip of the default action, the one Enter triggers.
pub fn key_primary(k: &str) -> Span<'static> {
    Span::styled(format!(" {k} "), Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD))
}

/// `[k] label` for hint bars.
pub fn hint(k: &str, label: &str) -> Vec<Span<'static>> {
    vec![key(k), Span::styled(format!(" {label}"), muted())]
}

pub fn hints(items: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut out = vec![];
    for (i, (k, l)) in items.iter().enumerate() {
        if i > 0 {
            out.push(Span::raw("   "));
        }
        out.extend(hint(k, l));
    }
    out
}

pub fn width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| s.content.width() as u16).sum()
}

pub fn fill(f: &mut Frame, rect: Rect, bg: Color) {
    let rect = rect.intersection(f.area());
    f.buffer_mut().set_style(rect, Style::new().bg(bg));
}

/// Pink bar left of a focused panel or block.
pub fn focus_bar(f: &mut Frame, x: u16, y: u16, h: u16) {
    for row in y..y + h {
        f.render_widget(Span::styled("▌", Style::new().fg(ACCENT)), Rect { x, y: row, width: 1, height: 1 });
    }
}

/// One-row section header: bold title (pink when focused), muted subtitle,
/// and `right` aligned to the right edge.
pub fn section(f: &mut Frame, rect: Rect, title: &str, sub: &str, right: Vec<Span<'static>>, focused: bool) {
    let title_style = if focused { bold().fg(ACCENT) } else { bold() };
    let mut left = vec![Span::styled(title.to_string(), title_style)];
    if !sub.is_empty() {
        left.push(Span::styled(format!("   {sub}"), muted()));
    }
    f.render_widget(Line::from(left), rect);
    let rw = width(&right);
    if rw > 0 && rw <= rect.width {
        f.render_widget(Line::from(right), Rect { x: rect.x + rect.width - rw, width: rw, ..rect });
    }
}

/// One row of a list in an overlay.
pub struct Item {
    /// Digit or letter that picks the row directly.
    pub key: Option<String>,
    /// Muted column before the label, e.g. an id.
    pub pre: String,
    pub label: String,
    /// Right-aligned, muted.
    pub meta: String,
    /// Optional coloured tag after the meta.
    pub tag: Option<(String, Color)>,
    pub dim: bool,
}

impl Item {
    pub fn new(label: impl Into<String>, meta: impl Into<String>) -> Self {
        Self { key: None, pre: String::new(), label: label.into(), meta: meta.into(), tag: None, dim: false }
    }
}

/// Draws list rows into `rect`, scrolled so `sel` is visible. Returns the
/// rectangle of every drawn row with its item index.
/// `active` lists highlight the chosen row strongly, others only mark it.
pub fn rows(f: &mut Frame, rect: Rect, items: &[Item], sel: usize, bg: Color, active: bool) -> Vec<(Rect, usize)> {
    let h = rect.height as usize;
    let start = if sel >= h { sel + 1 - h } else { 0 };
    let has_keys = items.iter().any(|i| i.key.is_some());
    let key_w = if has_keys { 4 } else { 0 };
    let meta_w = items.iter().map(|i| i.meta.width()).max().unwrap_or(0).min(rect.width as usize / 2);
    let tag_w = items.iter().filter_map(|i| i.tag.as_ref()).map(|(t, _)| t.width() + 2).max().unwrap_or(0);
    let pre_w = items.iter().map(|i| i.pre.width()).max().map_or(0, |w| if w > 0 { w + 2 } else { 0 });
    let mut hits = vec![];
    for (row, (i, it)) in items.iter().enumerate().skip(start).take(h).enumerate() {
        let selected = i == sel;
        let base = Style::new().bg(match (selected, active) {
            (true, true) => SELECT,
            (true, false) => KEY_BG,
            _ => bg,
        });
        let r = Rect { x: rect.x, y: rect.y + row as u16, width: rect.width, height: 1 };
        // Two columns of air between the label and the meta.
        let label_w = (rect.width as usize).saturating_sub(key_w + 1 + pre_w + 2 + meta_w + tag_w + 2);
        let mut spans = vec![];
        if has_keys {
            let k = it.key.clone().unwrap_or_default();
            spans.push(Span::styled(format!(" {:<2} ", k), base.fg(ACCENT).add_modifier(Modifier::BOLD)));
        }
        let label_style = if it.dim {
            base.fg(MUTED)
        } else if selected {
            base.add_modifier(Modifier::BOLD)
        } else {
            base
        };
        if pre_w > 0 {
            spans.push(Span::styled(format!(" {:>w$} ", it.pre, w = pre_w - 2), base.fg(MUTED)));
        }
        spans.push(Span::styled(format!(" {}  ", pad(&it.label, label_w)), label_style));
        let meta = truncate(&it.meta, meta_w);
        spans.push(Span::styled(format!("{meta:>meta_w$}  ", meta_w = meta_w), base.fg(MUTED)));
        if tag_w > 0 {
            let (t, c) = it.tag.clone().unwrap_or_default();
            spans.push(Span::styled(pad(&t, tag_w), base.fg(c)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(base), r);
        hits.push((r, i));
    }
    hits
}

/// A floating list: title, optional filter, rows, key hints. Used for the
/// menu, recent titles, search, team and player pickers.
pub struct Overlay<'a> {
    pub title: String,
    /// Right side of the title row, e.g. `крок 2/2`.
    pub aside: String,
    /// `(query, placeholder)` when the list can be filtered by typing.
    pub filter: Option<(&'a str, &'a str)>,
    pub items: Vec<Item>,
    pub sel: usize,
    /// Shown instead of rows when there are none.
    pub empty: String,
    pub footer: Vec<(&'a str, &'a str)>,
    pub width: u16,
}

pub struct OverlayLayout {
    pub rect: Rect,
    pub rows: Vec<(Rect, usize)>,
    pub cursor: Option<(u16, u16)>,
}

impl Overlay<'_> {
    pub fn draw(&self, f: &mut Frame, area: Rect) -> OverlayLayout {
        let w = self.width.min(area.width.saturating_sub(4));
        let fixed = 2 + 1 + u16::from(self.filter.is_some()) + 1 + 1 + 1; // padding, title, filter, gaps, footer
        let max_rows = area.height.saturating_sub(fixed + 2).max(1);
        let list_h = (self.items.len() as u16).clamp(1, max_rows);
        let h = fixed + list_h;
        let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + area.height.saturating_sub(h) / 3, width: w, height: h };
        f.render_widget(Clear, rect);
        f.render_widget(Block::new().style(Style::new().bg(OVERLAY)), rect);
        let ix = rect.x + 2;
        let iw = rect.width.saturating_sub(4);
        let mut y = rect.y + 1;

        section(f, Rect { x: ix, y, width: iw, height: 1 }, &self.title, "", vec![Span::styled(self.aside.clone(), muted())], true);
        y += 1;
        let mut cursor = None;
        if let Some((q, placeholder)) = self.filter {
            let text = if q.is_empty() { Span::styled(placeholder.to_string(), Style::new().fg(FAINT)) } else { Span::raw(q.to_string()) };
            f.render_widget(Line::from(vec![Span::styled("› ", Style::new().fg(ACCENT)), text]), Rect { x: ix, y, width: iw, height: 1 });
            cursor = Some((ix + 2 + q.width() as u16, y));
            y += 1;
        }
        y += 1;
        let list = Rect { x: ix - 1, y, width: iw + 2, height: list_h };
        let rows = if self.items.is_empty() {
            f.render_widget(Span::styled(self.empty.clone(), muted()), Rect { x: ix, y, width: iw, height: 1 });
            vec![]
        } else {
            rows(f, list, &self.items, self.sel, OVERLAY, true)
        };
        y += list_h + 1;
        f.render_widget(Line::from(hints(&self.footer)), Rect { x: ix, y, width: iw, height: 1 });
        OverlayLayout { rect, rows, cursor }
    }
}
