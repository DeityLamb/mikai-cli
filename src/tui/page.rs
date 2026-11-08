//! Full-screen watch page of one title.
//!
//! Left to right it follows the order of choice: track (voice/subs) → team →
//! episode → player → action. Key hints live inside the elements they drive.
//! Every action runs as a normal `/command`, so it also lands in the transcript.

use super::*;
use crate::play;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Releases,
    Episodes,
    Players,
    Buttons,
}

const FOCUS_ORDER: [Focus; 4] = [Focus::Releases, Focus::Episodes, Focus::Players, Focus::Buttons];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Btn {
    Mpv,
    Browser,
    Watched,
    Status,
    Site,
}

const BUTTONS: [Btn; 5] = [Btn::Mpv, Btn::Browser, Btn::Watched, Btn::Status, Btn::Site];

#[derive(Clone)]
pub(super) enum Hit {
    Focus(Focus),
    Back,
    Desc,
    Kind(String),
    Release(usize),
    Episode(usize),
    Provider(String),
    Button(Btn),
}

pub(super) struct Page {
    pub r: String,
    detail: AnimeDetail,
    focus: Focus,
    /// Track shown: `voice` or `sub`.
    kind: String,
    /// Index into the player's full release list.
    rel: usize,
    ep: usize,
    provider: Option<String>,
    btn: usize,
    full_desc: bool,
    mpv: bool,
    cols: usize,
    rel_scroll: usize,
    ep_scroll: usize,
    hits: Vec<(Rect, Hit)>,
    last_cmd: Option<u64>,
}

impl Page {
    pub fn new(detail: AnimeDetail, session: &Session) -> Self {
        let mut page = Self {
            r: session.r.clone(),
            detail,
            focus: Focus::Episodes,
            kind: "voice".into(),
            rel: 0,
            ep: 0,
            provider: None,
            btn: 0,
            full_desc: false,
            mpv: play::mpv_available(),
            cols: 1,
            rel_scroll: 0,
            ep_scroll: 0,
            hits: vec![],
            last_cmd: None,
        };
        if let Some(p) = &session.player {
            let remembered = session.team.as_ref().and_then(|t| p.releases.iter().position(|r| &r.id == t));
            page.rel = remembered.or_else(|| best_release(p, None, page.mpv)).unwrap_or(0);
            if let Some(r) = p.releases.get(page.rel) {
                page.kind = r.kind.clone();
            }
            page.ep = next_episode(p.releases.get(page.rel), session);
        }
        page
    }
}

/// The fullest release (of `kind`, if given), preferring ones mpv can play.
fn best_release(p: &Player, kind: Option<&str>, mpv: bool) -> Option<usize> {
    p.releases
        .iter()
        .enumerate()
        .filter(|(_, r)| kind.is_none_or(|k| r.kind == k))
        .max_by_key(|(i, r)| {
            let playable = mpv && r.episodes.iter().any(|e| e.sources.iter().any(|s| play::supports_mpv(&s.provider)));
            (playable, r.episodes.len(), std::cmp::Reverse(*i))
        })
        .map(|(i, _)| i)
}

/// Index of the first episode after what was watched or last opened.
fn next_episode(rel: Option<&Release>, s: &Session) -> usize {
    let Some(rel) = rel else { return 0 };
    let seen = watched_of(s).max(s.last_ep.unwrap_or(0.0));
    rel.episodes.iter().position(|e| e.number > seen).unwrap_or(rel.episodes.len().saturating_sub(1))
}

fn watched_of(s: &Session) -> f64 {
    s.entry.clone().flatten().and_then(|e| e.episodes).unwrap_or(0.0)
}

fn kind_title(kind: &str) -> &str {
    match kind {
        "voice" => "Озвучення",
        "sub" => "Субтитри",
        k => k,
    }
}

/// Clamps `scroll` so that `sel` stays inside a window of `h` rows.
fn follow(scroll: usize, sel: usize, h: usize) -> usize {
    let s = scroll.min(sel);
    if h > 0 && sel >= s + h { sel + 1 - h } else { s }
}

impl App {
    pub(super) fn open_page(&mut self, detail: AnimeDetail) {
        if let Some(s) = &self.session {
            self.page = Some(Page::new(detail, s));
        }
    }

    /// The page with its session and player, if the page still matches the session.
    fn page_parts(&self) -> Option<(&Page, &Session, Option<&Player>)> {
        let p = self.page.as_ref()?;
        let s = self.session.as_ref().filter(|s| s.r == p.r)?;
        Some((p, s, s.player.as_ref()))
    }

    /// Indices of the releases of the shown track.
    fn shown_releases(&self) -> Vec<usize> {
        let Some((p, _, Some(pl))) = self.page_parts() else { return vec![] };
        pl.releases.iter().enumerate().filter(|(_, r)| r.kind == p.kind).map(|(i, _)| i).collect()
    }

    fn current(&self) -> Option<(&Release, &Episode)> {
        let (p, _, player) = self.page_parts()?;
        let rel = player?.releases.get(p.rel)?;
        Some((rel, rel.episodes.get(p.ep)?))
    }

    /// The source that would play: the chosen provider if this episode has it,
    /// otherwise one mpv can open (when mpv is there), otherwise the first.
    fn current_source(&self) -> Option<&Source> {
        let (p, ..) = self.page_parts()?;
        let (_, ep) = self.current()?;
        p.provider
            .as_ref()
            .and_then(|name| ep.sources.iter().find(|s| &s.provider == name))
            .or_else(|| p.mpv.then(|| ep.sources.iter().find(|s| play::supports_mpv(&s.provider))).flatten())
            .or_else(|| ep.sources.first())
    }

    fn mpv_ready(&self) -> bool {
        self.page.as_ref().is_some_and(|p| p.mpv) && self.current_source().is_some_and(|s| play::supports_mpv(&s.provider))
    }

    fn page_run(&mut self, cmd: String) {
        self.submit(cmd);
        let id = self.entries.last().map(|e| e.id);
        if let Some(p) = &mut self.page {
            p.last_cmd = id;
        }
    }

    /// A local message in the result line, also kept in the transcript.
    fn page_note(&mut self, text: &str) {
        self.note(vec![SLine::of(text, Tone::Yellow)]);
        let id = self.entries.last().map(|e| e.id);
        if let Some(p) = &mut self.page {
            p.last_cmd = id;
        }
    }

    fn page_action(&mut self, b: Btn) {
        let Some((rel, ep)) = self.current() else { return };
        let (n, rel_id) = (ep.number, rel.id.clone());
        let provider = self.current_source().map(|s| s.provider.clone()).unwrap_or_default();
        let cmd = match b {
            Btn::Mpv => {
                if !self.page.as_ref().is_some_and(|p| p.mpv) {
                    self.page_note("mpv не встановлено");
                    return;
                }
                if !self.mpv_ready() {
                    self.page_note("цей плеєр не віддає пряме відео — оберіть плеєр з ◆ або браузер");
                    return;
                }
                format!("/play {} {rel_id} {provider}", fmt_num(n))
            }
            Btn::Browser => format!("/watch {} {rel_id} {provider}", fmt_num(n)),
            Btn::Watched => format!("/ep {}", fmt_num(n.floor())),
            Btn::Status => {
                let cur = self.session.as_ref().and_then(|s| s.entry.clone().flatten()).map(|e| e.status);
                let next = match cur.and_then(|c| LIST_STATUSES.iter().position(|s| *s == c)) {
                    Some(i) => LIST_STATUSES[(i + 1) % LIST_STATUSES.len()],
                    None => "watching",
                };
                format!("/status {next}")
            }
            Btn::Site => "/site".into(),
        };
        self.page_run(cmd);
    }

    fn primary(&mut self) {
        self.page_action(if self.mpv_ready() { Btn::Mpv } else { Btn::Browser });
    }

    fn select_release(&mut self, i: usize) {
        let Some((_, s, Some(player))) = self.page_parts() else { return };
        let Some(r) = player.releases.get(i) else { return };
        let (kind, ep) = (r.kind.clone(), next_episode(Some(r), s));
        if let Some(p) = &mut self.page {
            p.rel = i;
            p.ep = ep;
            p.kind = kind;
        }
    }

    fn move_release(&mut self, delta: i64) {
        let shown = self.shown_releases();
        let Some(p) = &self.page else { return };
        let pos = shown.iter().position(|&i| i == p.rel).unwrap_or(0) as i64;
        if let Some(&i) = shown.get((pos + delta).clamp(0, shown.len() as i64 - 1) as usize) {
            self.select_release(i);
        }
    }

    fn set_kind(&mut self, kind: &str) {
        let Some((p, s, Some(pl))) = self.page_parts() else { return };
        if p.kind == kind {
            return;
        }
        let remembered = s.team.as_ref().and_then(|t| pl.releases.iter().position(|r| &r.id == t && r.kind == kind));
        if let Some(i) = remembered.or_else(|| best_release(pl, Some(kind), p.mpv)) {
            self.select_release(i);
        }
    }

    fn toggle_kind(&mut self) {
        let Some(p) = &self.page else { return };
        let other = if p.kind == "voice" { "sub" } else { "voice" };
        self.set_kind(other);
    }

    fn move_provider(&mut self, delta: i64) {
        let Some((_, ep)) = self.current() else { return };
        let names: Vec<String> = ep.sources.iter().map(|s| s.provider.clone()).collect();
        if names.is_empty() {
            return;
        }
        let cur = self.current_source().map(|s| s.provider.clone());
        let i = cur.and_then(|c| names.iter().position(|n| *n == c)).unwrap_or(0) as i64;
        let next = names[(i + delta).rem_euclid(names.len() as i64) as usize].clone();
        if let Some(p) = &mut self.page {
            p.provider = Some(next);
        }
    }

    pub(super) fn page_key(&mut self, k: KeyEvent) {
        if self.page_parts().is_none() {
            self.page = None;
            return;
        }
        let ep_len = self.current().map(|(r, _)| r.episodes.len()).unwrap_or(0);
        let Some(p) = &mut self.page else { return };
        let focus = p.focus;
        let fi = FOCUS_ORDER.iter().position(|f| *f == focus).unwrap_or(0);
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.page = None,
            KeyCode::Tab => p.focus = FOCUS_ORDER[(fi + 1) % FOCUS_ORDER.len()],
            KeyCode::BackTab => p.focus = FOCUS_ORDER[(fi + FOCUS_ORDER.len() - 1) % FOCUS_ORDER.len()],
            KeyCode::Char('m') => self.page_action(Btn::Mpv),
            KeyCode::Char('b') => self.page_action(Btn::Browser),
            KeyCode::Char('w') => self.page_action(Btn::Watched),
            KeyCode::Char('s') => self.page_action(Btn::Status),
            KeyCode::Char('o') => self.page_action(Btn::Site),
            KeyCode::Char('t') => self.toggle_kind(),
            KeyCode::Char('1') => self.set_kind("voice"),
            KeyCode::Char('2') => self.set_kind("sub"),
            KeyCode::Char('c') => p.focus = Focus::Releases,
            KeyCode::Char('e') => p.focus = Focus::Episodes,
            KeyCode::Char('p') => p.focus = Focus::Players,
            KeyCode::Char('i') => p.full_desc = !p.full_desc,
            KeyCode::Enter | KeyCode::Char(' ') => match focus {
                Focus::Buttons => {
                    let b = BUTTONS[p.btn];
                    self.page_action(b);
                }
                _ => self.primary(),
            },
            code => match focus {
                Focus::Releases => match code {
                    KeyCode::Up | KeyCode::Char('k') => self.move_release(-1),
                    KeyCode::Down | KeyCode::Char('j') => self.move_release(1),
                    KeyCode::Right | KeyCode::Char('l') => p.focus = Focus::Episodes,
                    _ => {}
                },
                Focus::Episodes => {
                    let cols = p.cols.max(1);
                    let last = ep_len.saturating_sub(1);
                    match code {
                        KeyCode::Left | KeyCode::Char('h') if p.ep % cols == 0 => p.focus = Focus::Releases,
                        KeyCode::Left | KeyCode::Char('h') => p.ep -= 1,
                        KeyCode::Right | KeyCode::Char('l') if p.ep % cols == cols - 1 || p.ep == last => p.focus = Focus::Players,
                        KeyCode::Right | KeyCode::Char('l') => p.ep += 1,
                        KeyCode::Up | KeyCode::Char('k') => p.ep = p.ep.saturating_sub(cols),
                        KeyCode::Down | KeyCode::Char('j') if p.ep / cols == last / cols => p.focus = Focus::Buttons,
                        KeyCode::Down | KeyCode::Char('j') => p.ep = (p.ep + cols).min(last),
                        KeyCode::Home | KeyCode::Char('g') => p.ep = 0,
                        KeyCode::End | KeyCode::Char('G') => p.ep = last,
                        _ => {}
                    }
                }
                Focus::Players => match code {
                    KeyCode::Up | KeyCode::Char('k') => self.move_provider(-1),
                    KeyCode::Down | KeyCode::Char('j') => self.move_provider(1),
                    KeyCode::Left | KeyCode::Char('h') => p.focus = Focus::Episodes,
                    _ => {}
                },
                Focus::Buttons => match code {
                    KeyCode::Left | KeyCode::Char('h') => p.btn = p.btn.saturating_sub(1),
                    KeyCode::Right | KeyCode::Char('l') => p.btn = (p.btn + 1).min(BUTTONS.len() - 1),
                    KeyCode::Up | KeyCode::Char('k') => p.focus = Focus::Episodes,
                    _ => {}
                },
            },
        }
    }

    pub(super) fn page_click(&mut self, col: u16, row: u16) {
        let Some(p) = &self.page else { return };
        let Some(hit) = p.hits.iter().find(|(r, _)| r.contains((col, row).into())).map(|(_, h)| h.clone()) else { return };
        let set_focus = |app: &mut Self, f: Focus| {
            if let Some(p) = &mut app.page {
                p.focus = f;
            }
        };
        match hit {
            Hit::Back => self.page = None,
            Hit::Focus(fo) => set_focus(self, fo),
            Hit::Desc => {
                if let Some(p) = &mut self.page {
                    p.full_desc = !p.full_desc;
                }
            }
            Hit::Kind(k) => self.set_kind(&k),
            Hit::Release(i) => {
                self.select_release(i);
                set_focus(self, Focus::Releases);
            }
            Hit::Episode(i) => {
                // A second click on the selected episode plays it.
                let again = self.page.as_ref().is_some_and(|p| p.ep == i && p.focus == Focus::Episodes);
                if let Some(p) = &mut self.page {
                    p.ep = i;
                }
                set_focus(self, Focus::Episodes);
                if again {
                    self.primary();
                }
            }
            Hit::Provider(name) => {
                if let Some(p) = &mut self.page {
                    p.provider = Some(name);
                }
                set_focus(self, Focus::Players);
            }
            Hit::Button(b) => {
                if let Some(p) = &mut self.page {
                    p.btn = BUTTONS.iter().position(|x| *x == b).unwrap_or(0);
                }
                set_focus(self, Focus::Buttons);
                self.page_action(b);
            }
        }
    }

    pub(super) fn page_hints(&self) -> &'static str {
        "tab колонки · ctrl+h нещодавні · ctrl+p меню · esc назад"
    }

    // ---------- drawing ----------

    pub(super) fn draw_page(&mut self, f: &mut Frame, area: Rect) {
        if self.page_parts().is_none() {
            self.page = None;
            return;
        }
        let mut hits: Vec<(Rect, Hit)> = vec![];
        let x = area.x + 2;
        let w = area.width.saturating_sub(4);
        let bottom = area.y + area.height;
        let mut y = area.y + 1;

        y = self.draw_info(f, x, y, w, bottom, &mut hits);

        let (page, _, player) = self.page_parts().unwrap();
        if page.detail.licensed {
            f.render_widget(
                Line::styled("Ліцензовано в Україні — дивіться легально:  o mikai.me", Style::new().fg(Color::Yellow)),
                Rect { x, y, width: w, height: 1 },
            );
            hits.push((Rect { x, y, width: w, height: 1 }, Hit::Button(Btn::Site)));
            self.page.as_mut().unwrap().hits = hits;
            return;
        }
        if player.is_none() {
            f.render_widget(Line::styled("завантаження серій…", dim()), Rect { x, y, width: w, height: 1 });
            self.page.as_mut().unwrap().hits = hits;
            return;
        }

        // Three pickers: team | episode | player, each on its own panel.
        let buttons_h = 4;
        let rel_w = 30.min(w / 3);
        let prov_w = 20.min(w / 5);
        let gap = 3;
        let ep_w = w.saturating_sub(rel_w + prov_w + 2 * gap);
        // As tall as the longest list, but within the screen.
        let avail = bottom.saturating_sub(y + 6 + buttons_h).max(3) as usize;
        let list_h = self.picker_rows(ep_w).clamp(3, avail);
        let focus = self.page.as_ref().map(|p| p.focus).unwrap_or(Focus::Episodes);
        for (cx, cw, fo) in [(x, rel_w, Focus::Releases), (x + rel_w + gap, ep_w, Focus::Episodes), (x + rel_w + ep_w + 2 * gap, prov_w, Focus::Players)] {
            let bg = if focus == fo { PANEL_FOCUS } else { PANEL_BG };
            fill(f, Rect { x: cx - 1, y, width: cw + 2, height: list_h as u16 + 6 }, bg);
        }
        y += 1;
        let rel_x = x;
        let ep_x = rel_x + rel_w + gap;
        let prov_x = ep_x + ep_w + gap;

        let (rel_scroll, ep_scroll, cols) = self.draw_pickers(
            f,
            [(rel_x, rel_w), (ep_x, ep_w), (prov_x, prov_w)],
            y,
            list_h,
            &mut hits,
        );
        y += 4 + list_h as u16 + 2;

        self.draw_buttons(f, x, y, w, &mut hits);
        y += 2;
        self.draw_result(f, x, y, w);

        if let Some(p) = &mut self.page {
            p.hits = hits;
            p.cols = cols;
            p.rel_scroll = rel_scroll;
            p.ep_scroll = ep_scroll;
        }
    }

    /// Title, meta and description. Returns the next free row.
    fn draw_info(&self, f: &mut Frame, x: u16, y: u16, w: u16, bottom: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        let next = self.draw_info_inner(f, x, y, w, bottom, hits);
        // Panel behind the info, one row of padding around it.
        let rect = Rect { x: x - 1, y: y - 1, width: w + 2, height: next - y + 1 };
        f.buffer_mut().set_style(rect, Style::new().bg(PANEL_BG));
        next + 2
    }

    fn draw_info_inner(&self, f: &mut Frame, x: u16, mut y: u16, w: u16, bottom: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        let (page, session, _) = self.page_parts().unwrap();
        let d = &page.detail;
        let indent = "       ";

        let entry = session.entry.clone().flatten();
        let status = match &entry {
            Some(e) => format!(
                "● {} · {}/{}",
                list_status_label(&e.status),
                e.episodes.map(fmt_num).unwrap_or_else(|| "0".into()),
                d.episodes.total.filter(|&t| t > 0).map(|t| t.to_string()).unwrap_or_else(|| "?".into())
            ),
            None if self.client.has_key() => "○ немає в списках".into(),
            None => String::new(),
        };
        let mut title = vec![
            Span::styled("‹ ", Style::new().fg(ACCENT)),
            Span::styled("Esc  ", dim()),
            Span::styled(d.titles.main().to_string(), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        ];
        let used: usize = title.iter().map(|s| s.content.width()).sum();
        title.push(Span::raw(" ".repeat((w as usize).saturating_sub(used + status.width()))));
        title.push(Span::styled(status, Style::new().fg(Color::Green)));
        f.render_widget(Line::from(title), Rect { x, y, width: w, height: 1 });
        hits.push((Rect { x, y, width: 6, height: 1 }, Hit::Back));
        y += 1;

        let mut rendered = vec![];
        let card = view::anime_card(d, None, false);
        // The card starts with the title; keep the lines up to the description.
        for l in card.iter().skip(1).take_while(|l| !l.segs.is_empty()) {
            wrap_line(l, w as usize, indent, &mut rendered);
        }
        let desc_top = rendered.len() + 1;
        let mut desc_rows = 0;
        if let Some(desc) = &d.description {
            let mut lines = vec![];
            for p in desc.trim().lines().filter(|l| !l.trim().is_empty()) {
                wrap_line(&SLine::of(p, Tone::Plain), w as usize, indent, &mut lines);
            }
            let max = if page.full_desc { lines.len() } else { 2 };
            let more = lines.len() > max;
            rendered.push(Line::raw(""));
            desc_rows = lines.len().min(max) + 1;
            rendered.extend(lines.into_iter().take(max));
            let hint = if more { "ще" } else if page.full_desc { "згорнути" } else { "" };
            if !hint.is_empty() {
                rendered.push(Line::from(vec![Span::raw(indent), key_span("i"), Span::styled(format!(" {hint}"), dim())]));
            } else {
                desc_rows -= 1;
            }
        }
        // Leave room for the pickers.
        let max_h = bottom.saturating_sub(y + 16) as usize;
        let h = rendered.len().min(max_h.max(3));
        f.render_widget(Paragraph::new(rendered.into_iter().take(h).collect::<Vec<_>>()), Rect { x, y, width: w, height: h as u16 });
        if desc_rows > 0 {
            hits.push((Rect { x, y: y + desc_top as u16, width: w, height: desc_rows as u16 }, Hit::Desc));
        }
        y + h as u16
    }

    /// Voice/subs switch at the top of the team column.
    fn draw_kinds(&self, f: &mut Frame, x: u16, y: u16, w: u16, hits: &mut Vec<(Rect, Hit)>) {
        let (page, _, Some(pl)) = self.page_parts().unwrap() else { return };
        let mut spans = vec![];
        let mut cx = x;
        for kind in ["voice", "sub"] {
            let n = pl.releases.iter().filter(|r| r.kind == kind).count();
            let text = format!(" {} {n} ", kind_title(kind));
            let style = if page.kind == kind {
                Style::new().bg(SELECTED).add_modifier(Modifier::BOLD)
            } else if n == 0 {
                dim()
            } else {
                Style::new().fg(Color::Gray)
            };
            let tw = text.width() as u16;
            if n > 0 {
                hits.push((Rect { x: cx, y, width: tw, height: 1 }, Hit::Kind(kind.into())));
            }
            spans.push(Span::styled(text, style));
            cx += tw;
        }
        let key = Span::styled(" t ", Style::new().bg(CHIP_BG).fg(ACCENT).add_modifier(Modifier::BOLD));
        let used = (cx - x) as usize;
        spans.push(Span::raw(" ".repeat((w as usize).saturating_sub(used + 3))));
        hits.push((Rect { x: x + w.saturating_sub(3), y, width: 3, height: 1 }, Hit::Kind(if page.kind == "voice" { "sub".into() } else { "voice".into() })));
        spans.push(key);
        f.render_widget(Line::from(spans), Rect { x, y, width: w, height: 1 });
    }

    fn picker_rows(&self, ep_w: u16) -> usize {
        let Some((page, _, Some(pl))) = self.page_parts() else { return 3 };
        let teams = self.shown_releases().len();
        let eps = pl.releases.get(page.rel).map_or(0, |rel| {
            let cell_w = rel.episodes.iter().map(|e| e.label().width()).max().unwrap_or(1) + 4;
            rel.episodes.len().div_ceil((ep_w as usize / cell_w).max(1))
        });
        let players = self.current().map_or(0, |(_, e)| e.sources.len());
        teams.max(eps).max(players)
    }

    /// Returns the scroll offsets and grid columns it used.
    fn draw_pickers(
        &self,
        f: &mut Frame,
        cols_xw: [(u16, u16); 3],
        y: u16,
        list_h: usize,
        hits: &mut Vec<(Rect, Hit)>,
    ) -> (usize, usize, usize) {
        let (page, session, Some(pl)) = self.page_parts().unwrap() else { return (0, 0, 1) };
        let [(rel_x, rel_w), (ep_x, ep_w), (prov_x, prov_w)] = cols_xw;
        let focus = page.focus;
        // The team column has the voice/subs switch under its header; lists line up below it.
        self.draw_kinds(f, rel_x, y + 2, rel_w, hits);
        let list_y = y + 4;

        let mut header = |key: &str, label: &str, fo: Focus, x: u16, w: u16| {
            let spans = chip(key, label, focus == fo, false);
            let tw = spans.iter().map(|s| s.content.width()).sum::<usize>() as u16;
            f.render_widget(Line::from(spans), Rect { x, y, width: w, height: 1 });
            hits.push((Rect { x, y, width: tw.min(w), height: 1 }, Hit::Focus(fo)));
        };
        header("c", "Команда", Focus::Releases, rel_x, rel_w);
        header("e", "Серія", Focus::Episodes, ep_x, ep_w);
        header("p", "Плеєр", Focus::Players, prov_x, prov_w);
        let item_style = |sel: bool, on: bool| match (sel, on) {
            (true, true) => Style::new().bg(SELECTED).add_modifier(Modifier::BOLD),
            (true, false) => Style::new().bg(Color::Rgb(56, 48, 64)),
            _ => Style::new(),
        };

        // Teams of the shown track.
        let shown = self.shown_releases();
        let pos = shown.iter().position(|&i| i == page.rel).unwrap_or(0);
        let rel_scroll = follow(page.rel_scroll, pos, list_h);
        for (row, &i) in shown.iter().enumerate().skip(rel_scroll).take(list_h) {
            let r = &pl.releases[i];
            let style = item_style(i == page.rel, focus == Focus::Releases);
            let count = r.last_episode.map(fmt_num).unwrap_or_else(|| "–".into());
            let name_w = (rel_w as usize).saturating_sub(7);
            let rect = Rect { x: rel_x, y: list_y + (row - rel_scroll) as u16, width: rel_w, height: 1 };
            f.render_widget(
                Line::from(vec![
                    Span::styled(format!(" {}", crate::style::pad(&r.team_names(), name_w)), style),
                    Span::styled(format!("{count:>4} "), style.fg(Color::DarkGray)),
                ]),
                rect,
            );
            hits.push((rect, Hit::Release(i)));
        }
        if shown.len() > list_h {
            f.render_widget(
                Line::styled(format!("{}/{}", pos + 1, shown.len()), dim()).right_aligned(),
                Rect { x: rel_x, y, width: rel_w, height: 1 },
            );
        }

        // Episode grid.
        let mut cols = 1;
        let mut ep_scroll = 0;
        if let Some(rel) = pl.releases.get(page.rel) {
            let watched = watched_of(session);
            let cell_w = rel.episodes.iter().map(|e| e.label().width()).max().unwrap_or(1) + 4;
            cols = (ep_w as usize / cell_w).max(1);
            let rows_total = rel.episodes.len().div_ceil(cols);
            ep_scroll = follow(page.ep_scroll, page.ep / cols, list_h);
            for row in ep_scroll..rows_total.min(ep_scroll + list_h) {
                let ry = list_y + (row - ep_scroll) as u16;
                for col in 0..cols {
                    let i = row * cols + col;
                    let Some(e) = rel.episodes.get(i) else { break };
                    let seen = e.number <= watched;
                    let opened = session.last_ep == Some(e.number);
                    let mark = if seen { "✓" } else if opened { "▸" } else { " " };
                    let mut style = if seen {
                        Style::new().fg(Color::Green)
                    } else if opened {
                        Style::new().fg(Color::Yellow)
                    } else {
                        Style::new()
                    };
                    if i == page.ep {
                        style = style.patch(item_style(true, focus == Focus::Episodes));
                    }
                    let rect = Rect { x: ep_x + (col * cell_w) as u16, y: ry, width: cell_w as u16 - 1, height: 1 };
                    f.render_widget(Span::styled(format!("{mark} {} ", crate::style::pad(&e.label(), cell_w - 4)), style), rect);
                    hits.push((rect, Hit::Episode(i)));
                }
            }
            if rows_total > list_h {
                f.render_widget(
                    Line::styled(format!("{}/{}", page.ep + 1, rel.episodes.len()), dim()).right_aligned(),
                    Rect { x: ep_x, y, width: ep_w, height: 1 },
                );
            }
        }

        // Players of the selected episode.
        let cur = self.current_source().map(|s| s.provider.clone());
        if let Some((_, ep)) = self.current() {
            for (row, s) in ep.sources.iter().enumerate().take(list_h) {
                let sel = cur.as_deref() == Some(s.provider.as_str());
                let style = item_style(sel, focus == Focus::Players);
                let mpv = if play::supports_mpv(&s.provider) { "◆ mpv" } else { "" };
                let name_w = (prov_w as usize).saturating_sub(7);
                let rect = Rect { x: prov_x, y: list_y + row as u16, width: prov_w, height: 1 };
                f.render_widget(
                    Line::from(vec![
                        Span::styled(format!(" {}", crate::style::pad(&s.provider, name_w)), style),
                        Span::styled(format!("{mpv:>5} "), style.fg(ACCENT)),
                    ]),
                    rect,
                );
                hits.push((rect, Hit::Provider(s.provider.clone())));
            }
        }
        (rel_scroll, ep_scroll, cols)
    }

    fn draw_buttons(&self, f: &mut Frame, x: u16, y: u16, _w: u16, hits: &mut Vec<(Rect, Hit)>) {
        let (page, session, _) = self.page_parts().unwrap();
        let mpv_ok = self.mpv_ready();
        let ep = self.current().map(|(_, e)| fmt_num(e.number.floor())).unwrap_or_default();
        let entry = session.entry.clone().flatten();
        let mut bx = x;
        for (i, b) in BUTTONS.iter().enumerate() {
            let primary = (*b == Btn::Mpv && mpv_ok) || (*b == Btn::Browser && !mpv_ok);
            let (key, label) = match b {
                Btn::Mpv => (if mpv_ok { "⏎" } else { "m" }, "mpv".to_string()),
                Btn::Browser => (if mpv_ok { "b" } else { "⏎" }, "браузер".to_string()),
                Btn::Watched => ("w", format!("переглянуто {ep}")),
                Btn::Status => (
                    "s",
                    match &entry {
                        Some(e) => format!("список: {}", list_status_label(&e.status)),
                        None => "до списку".into(),
                    },
                ),
                Btn::Site => ("o", "mikai.me".to_string()),
            };
            let disabled = *b == Btn::Mpv && !mpv_ok;
            let focused = page.focus == Focus::Buttons && page.btn == i;
            // Highlighted: the focused button, or the primary one when buttons are not focused.
            let active = focused || (primary && page.focus != Focus::Buttons);
            let spans = chip(key, &label, active, disabled);
            let tw = spans.iter().map(|s| s.content.width()).sum::<usize>() as u16;
            let rect = Rect { x: bx, y, width: tw, height: 1 };
            f.render_widget(Line::from(spans), rect);
            hits.push((rect, Hit::Button(*b)));
            bx += tw + 2;
        }
    }

    fn draw_result(&self, f: &mut Frame, x: u16, y: u16, w: u16) {
        let Some(page) = &self.page else { return };
        let Some(e) = page.last_cmd.and_then(|id| self.entries.iter().rev().find(|e| e.id == id)) else { return };
        let mut out = vec![];
        if e.pending {
            out.push(Line::from(vec![
                Span::styled(SPINNER[self.tick % SPINNER.len()], Style::new().fg(ACCENT)),
                Span::styled(format!(" {}", e.cmd), dim()),
            ]));
        } else if let Some(l) = e.lines.iter().find(|l| !l.segs.is_empty()) {
            wrap_line(&l.clone().nowrap(), w as usize, "", &mut out);
        }
        f.render_widget(Paragraph::new(out), Rect { x, y, width: w, height: 1 });
    }
}

const PANEL_BG: Color = Color::Rgb(26, 25, 32);
const PANEL_FOCUS: Color = Color::Rgb(36, 32, 44);
const CHIP_BG: Color = Color::Rgb(44, 41, 53);

fn fill(f: &mut Frame, rect: Rect, bg: Color) {
    let area = f.area();
    f.buffer_mut().set_style(rect.intersection(area), Style::new().bg(bg));
}

/// A key-labelled chip like ` 1  Озвучення 8 `: the key on its own tint.
fn chip(key: &str, label: &str, active: bool, disabled: bool) -> Vec<Span<'static>> {
    let (key_bg, bg) = if active { (ACCENT, SELECTED) } else { (CHIP_BG, CHIP_BG) };
    let key_fg = if active { Color::Black } else { ACCENT };
    let fg = if disabled { Color::DarkGray } else { Color::Reset };
    let mut text = Style::new().bg(bg).fg(fg);
    if active {
        text = text.add_modifier(Modifier::BOLD);
    }
    vec![
        Span::styled(format!(" {key} "), Style::new().bg(key_bg).fg(key_fg).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {label} "), text),
    ]
}

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn key_span(k: &str) -> Span<'static> {
    Span::styled(k.to_string(), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
}
