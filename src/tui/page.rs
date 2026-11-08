//! Full-screen watch page of one title: info, release and episode pickers,
//! player choice and actions. Every action runs as a normal `/command`, so it
//! also lands in the transcript.

use super::*;
use crate::play;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Releases,
    Episodes,
    Players,
    Buttons,
}

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
    Back,
    Desc,
    Release(usize),
    Episode(usize),
    Provider(String),
    Button(Btn),
}

pub(super) struct Page {
    pub r: String,
    detail: AnimeDetail,
    focus: Focus,
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
            let mpv = page.mpv;
            // The team used before; otherwise the fullest release, preferring
            // ones mpv can play when mpv is there.
            page.rel = session
                .team
                .as_ref()
                .and_then(|t| p.releases.iter().position(|r| &r.id == t))
                .or_else(|| {
                    p.releases
                        .iter()
                        .enumerate()
                        .max_by_key(|(i, r)| {
                            let playable = mpv && r.episodes.iter().any(|e| e.sources.iter().any(|s| play::supports_mpv(&s.provider)));
                            (playable, r.episodes.len(), std::cmp::Reverse(*i))
                        })
                        .map(|(i, _)| i)
                })
                .unwrap_or(0);
            page.ep = next_episode(p.releases.get(page.rel), session);
        }
        page
    }
}

/// Index of the first episode after what was watched or last opened.
fn next_episode(rel: Option<&Release>, s: &Session) -> usize {
    let Some(rel) = rel else { return 0 };
    let watched = s.entry.clone().flatten().and_then(|e| e.episodes).unwrap_or(0.0);
    let seen = watched.max(s.last_ep.unwrap_or(0.0));
    rel.episodes.iter().position(|e| e.number > seen).unwrap_or(rel.episodes.len().saturating_sub(1))
}

fn watched_of(s: &Session) -> f64 {
    s.entry.clone().flatten().and_then(|e| e.episodes).unwrap_or(0.0)
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
                    self.page_note("цей плеєр не віддає пряме відео — оберіть ashdi (p) або браузер (b)");
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

    /// A local message in the result line, also kept in the transcript.
    fn page_note(&mut self, text: &str) {
        self.note(vec![SLine::of(text, Tone::Yellow)]);
        let id = self.entries.last().map(|e| e.id);
        if let Some(p) = &mut self.page {
            p.last_cmd = id;
        }
    }

    fn select_release(&mut self, i: usize) {
        let Some((_, s, Some(player))) = self.page_parts() else { return };
        let i = i.min(player.releases.len().saturating_sub(1));
        let ep = next_episode(player.releases.get(i), s);
        if let Some(p) = &mut self.page {
            p.rel = i;
            p.ep = ep;
        }
    }

    fn cycle_provider(&mut self, delta: i64) {
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
        let rel_len = self.page_parts().and_then(|(_, _, pl)| pl).map(|pl| pl.releases.len()).unwrap_or(0);
        let ep_len = self.current().map(|(r, _)| r.episodes.len()).unwrap_or(0);
        let Some(p) = &mut self.page else { return };
        let focus = p.focus;
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.page = None,
            KeyCode::Tab | KeyCode::BackTab => {
                let order = [Focus::Releases, Focus::Episodes, Focus::Players, Focus::Buttons];
                let i = order.iter().position(|f| *f == focus).unwrap_or(0);
                let d = if k.code == KeyCode::Tab { 1 } else { order.len() - 1 };
                p.focus = order[(i + d) % order.len()];
            }
            KeyCode::Char('m') => self.page_action(Btn::Mpv),
            KeyCode::Char('b') => self.page_action(Btn::Browser),
            KeyCode::Char('w') => self.page_action(Btn::Watched),
            KeyCode::Char('s') => self.page_action(Btn::Status),
            KeyCode::Char('o') => self.page_action(Btn::Site),
            KeyCode::Char('p') => self.cycle_provider(1),
            KeyCode::Char('i') => p.full_desc = !p.full_desc,
            KeyCode::Enter | KeyCode::Char(' ') => match focus {
                Focus::Releases => p.focus = Focus::Episodes,
                Focus::Episodes | Focus::Players => self.primary(),
                Focus::Buttons => {
                    let b = BUTTONS[p.btn];
                    self.page_action(b);
                }
            },
            code => match focus {
                Focus::Releases => match code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        let i = p.rel.saturating_sub(1);
                        self.select_release(i);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let i = (p.rel + 1).min(rel_len.saturating_sub(1));
                        self.select_release(i);
                    }
                    KeyCode::Right | KeyCode::Char('l') => p.focus = Focus::Episodes,
                    _ => {}
                },
                Focus::Episodes => {
                    let cols = p.cols.max(1);
                    let last = ep_len.saturating_sub(1);
                    match code {
                        KeyCode::Left | KeyCode::Char('h') if p.ep % cols == 0 => p.focus = Focus::Releases,
                        KeyCode::Left | KeyCode::Char('h') => p.ep -= 1,
                        KeyCode::Right | KeyCode::Char('l') => p.ep = (p.ep + 1).min(last),
                        KeyCode::Up | KeyCode::Char('k') => p.ep = p.ep.saturating_sub(cols),
                        KeyCode::Down | KeyCode::Char('j') if p.ep + cols > last && p.ep / cols == last / cols => {
                            p.focus = Focus::Players
                        }
                        KeyCode::Down | KeyCode::Char('j') => p.ep = (p.ep + cols).min(last),
                        KeyCode::Home | KeyCode::Char('g') => p.ep = 0,
                        KeyCode::End | KeyCode::Char('G') => p.ep = last,
                        _ => {}
                    }
                }
                Focus::Players => match code {
                    KeyCode::Left | KeyCode::Char('h') => self.cycle_provider(-1),
                    KeyCode::Right | KeyCode::Char('l') => self.cycle_provider(1),
                    KeyCode::Up | KeyCode::Char('k') => p.focus = Focus::Episodes,
                    KeyCode::Down | KeyCode::Char('j') => p.focus = Focus::Buttons,
                    _ => {}
                },
                Focus::Buttons => match code {
                    KeyCode::Left | KeyCode::Char('h') => p.btn = p.btn.saturating_sub(1),
                    KeyCode::Right | KeyCode::Char('l') => p.btn = (p.btn + 1).min(BUTTONS.len() - 1),
                    KeyCode::Up | KeyCode::Char('k') => p.focus = Focus::Players,
                    _ => {}
                },
            },
        }
    }

    pub(super) fn page_click(&mut self, col: u16, row: u16) {
        let Some(p) = &self.page else { return };
        let Some(hit) = p.hits.iter().find(|(r, _)| r.contains((col, row).into())).map(|(_, h)| h.clone()) else { return };
        match hit {
            Hit::Back => self.page = None,
            Hit::Desc => {
                if let Some(p) = &mut self.page {
                    p.full_desc = !p.full_desc;
                }
            }
            Hit::Release(i) => {
                self.select_release(i);
                if let Some(p) = &mut self.page {
                    p.focus = Focus::Releases;
                }
            }
            Hit::Episode(i) => {
                let again = self.page.as_ref().is_some_and(|p| p.ep == i && p.focus == Focus::Episodes);
                if let Some(p) = &mut self.page {
                    p.ep = i;
                    p.focus = Focus::Episodes;
                }
                // A second click on the selected episode plays it.
                if again {
                    self.primary();
                }
            }
            Hit::Provider(name) => {
                if let Some(p) = &mut self.page {
                    p.provider = Some(name);
                    p.focus = Focus::Players;
                }
            }
            Hit::Button(b) => {
                if let Some(p) = &mut self.page {
                    p.btn = BUTTONS.iter().position(|x| *x == b).unwrap_or(0);
                    p.focus = Focus::Buttons;
                }
                self.page_action(b);
            }
        }
    }

    // ---------- drawing ----------

    pub(super) fn draw_page(&mut self, f: &mut Frame, area: Rect) {
        let Some((page, session, player)) = self.page_parts() else {
            self.page = None;
            return;
        };
        let mut hits: Vec<(Rect, Hit)> = vec![];
        let d = &page.detail;
        let x = area.x + 2;
        let w = area.width.saturating_sub(4);
        let mut y = area.y + 1;
        let dim_s = Style::new().fg(Color::DarkGray);
        let line = |f: &mut Frame, y: u16, l: Line| f.render_widget(l, Rect { x, y, width: w, height: 1 });

        // Title row.
        let back = Span::styled("‹ Esc  ", dim_s);
        let mut title = vec![back, Span::styled(d.titles.main().to_string(), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))];
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
        let used: usize = title.iter().map(|s| s.content.width()).sum();
        title.push(Span::raw(" ".repeat((w as usize).saturating_sub(used + status.width()))));
        title.push(Span::styled(status, Style::new().fg(Color::Green)));
        line(f, y, Line::from(title));
        hits.push((Rect { x, y, width: 6, height: 1 }, Hit::Back));
        y += 1;
        let indent = "       ";
        if let Some(sec) = d.titles.secondary() {
            line(f, y, Line::styled(format!("{indent}{sec}"), dim_s));
            y += 1;
        }
        y += 1;

        // Info lines.
        let mut info: Lines = vec![];
        let card = view::anime_card(d, None, false);
        // Meta, scores and genres are lines 1..: skip title/secondary, stop at the description.
        for l in card.iter().skip(if d.titles.secondary().is_some() { 2 } else { 1 }) {
            if l.segs.is_empty() {
                break;
            }
            info.push(l.clone());
        }
        let mut rendered = vec![];
        for l in &info {
            wrap_line(l, w as usize, indent, &mut rendered);
        }
        if let Some(desc) = &d.description {
            let mut desc_lines = vec![];
            for p in desc.trim().lines().filter(|l| !l.trim().is_empty()) {
                wrap_line(&SLine::of(p, Tone::Plain), w as usize, indent, &mut desc_lines);
            }
            let max = if page.full_desc { desc_lines.len() } else { 3 };
            let cut = desc_lines.len() > max;
            rendered.push(Line::raw(""));
            let desc_y = y + rendered.len() as u16;
            let shown = desc_lines.len().min(max);
            rendered.extend(desc_lines.into_iter().take(max));
            if cut {
                rendered.push(Line::styled(format!("{indent}… ще (i)"), dim_s));
            } else if page.full_desc {
                rendered.push(Line::styled(format!("{indent}згорнути (i)"), dim_s));
            }
            hits.push((Rect { x, y: desc_y, width: w, height: shown as u16 + 1 }, Hit::Desc));
        }
        // Keep at least 8 rows for the pickers.
        let footer_h = 7u16;
        let max_info = (area.y + area.height).saturating_sub(y + footer_h + 8) as usize;
        let info_h = rendered.len().min(max_info);
        f.render_widget(Paragraph::new(rendered.into_iter().take(info_h).collect::<Vec<_>>()), Rect { x, y, width: w, height: info_h as u16 });
        y += info_h as u16 + 1;

        if d.licensed {
            line(f, y, Line::styled(format!("{indent}Ліцензовано в Україні — дивіться легально (o — сторінка на mikai.me)"), Style::new().fg(Color::Yellow)));
            self.page.as_mut().unwrap().hits = hits;
            return;
        }

        // Pickers.
        let pick_h = (area.y + area.height).saturating_sub(y + footer_h).max(3);
        let rel_w = 30.min(w / 3);
        let rel_x = x;
        let ep_x = x + rel_w + 3;
        let ep_w = w.saturating_sub(rel_w + 3);
        let focus = page.focus;
        let head = |label: String, on: bool| {
            Line::styled(label, if on { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new().add_modifier(Modifier::BOLD) })
        };
        f.render_widget(head("Озвучення та субтитри".into(), focus == Focus::Releases), Rect { x: rel_x, y, width: rel_w, height: 1 });

        let Some(player) = player else {
            f.render_widget(Line::styled("завантаження…", dim_s), Rect { x: rel_x, y: y + 2, width: rel_w, height: 1 });
            self.page.as_mut().unwrap().hits = hits;
            return;
        };
        let watched = watched_of(session);
        let rel = player.releases.get(page.rel);
        let ep_head = rel
            .map(|r| format!("Серії · {} · {}", r.team_names(), r.kind_label()))
            .unwrap_or_else(|| "Серії".into());
        f.render_widget(head(ep_head, focus == Focus::Episodes), Rect { x: ep_x, y, width: ep_w, height: 1 });
        let list_y = y + 2;
        let list_h = pick_h.saturating_sub(2) as usize;

        // Releases.
        let mut rel_scroll = page.rel_scroll.min(page.rel);
        if page.rel >= rel_scroll + list_h {
            rel_scroll = page.rel + 1 - list_h;
        }
        for (row, (i, r)) in player.releases.iter().enumerate().skip(rel_scroll).take(list_h).enumerate() {
            let sel = i == page.rel;
            let style = match (sel, focus == Focus::Releases) {
                (true, true) => Style::new().bg(SELECTED).add_modifier(Modifier::BOLD),
                (true, false) => Style::new().bg(Color::Rgb(40, 38, 48)),
                _ => Style::new(),
            };
            let kind = if r.kind == "sub" { "суб" } else { "озв" };
            let count = r.last_episode.map(fmt_num).unwrap_or_else(|| "–".into());
            let name_w = (rel_w as usize).saturating_sub(10);
            let l = Line::from(vec![
                Span::styled(format!(" {} ", crate::style::pad(&r.team_names(), name_w)), style),
                Span::styled(format!("{kind} {:>3} ", count), style.fg(Color::DarkGray)),
            ]);
            let rect = Rect { x: rel_x, y: list_y + row as u16, width: rel_w, height: 1 };
            f.render_widget(l, rect);
            hits.push((rect, Hit::Release(i)));
        }

        // Episode grid.
        let mut cols = 1;
        let mut new_ep_scroll = page.ep_scroll;
        if let Some(rel) = rel {
            let cell_w = rel.episodes.iter().map(|e| e.label().width()).max().unwrap_or(1) + 4;
            cols = (ep_w as usize / cell_w).max(1);
            let rows_total = rel.episodes.len().div_ceil(cols);
            let sel_row = page.ep / cols;
            let mut ep_scroll = page.ep_scroll.min(sel_row);
            if sel_row >= ep_scroll + list_h {
                ep_scroll = sel_row + 1 - list_h;
            }
            let last_ep = session.last_ep;
            for row in ep_scroll..rows_total.min(ep_scroll + list_h) {
                let mut spans = vec![];
                for col in 0..cols {
                    let i = row * cols + col;
                    let Some(e) = rel.episodes.get(i) else { break };
                    let seen = e.number <= watched;
                    let opened = last_ep == Some(e.number);
                    let mark = if seen { "✓" } else if opened { "▸" } else { " " };
                    let text = format!("{mark} {}", crate::style::pad(&e.label(), cell_w - 4));
                    let mut style = if seen { Style::new().fg(Color::Green) } else if opened { Style::new().fg(Color::Yellow) } else { Style::new() };
                    if i == page.ep {
                        style = if focus == Focus::Episodes {
                            style.bg(SELECTED).add_modifier(Modifier::BOLD)
                        } else {
                            style.bg(Color::Rgb(40, 38, 48))
                        };
                    }
                    let cx = ep_x + (col * cell_w) as u16;
                    hits.push((Rect { x: cx, y: list_y + (row - ep_scroll) as u16, width: cell_w as u16 - 1, height: 1 }, Hit::Episode(i)));
                    spans.push(Span::styled(format!("{text} "), style));
                    spans.push(Span::raw(" "));
                }
                f.render_widget(Line::from(spans), Rect { x: ep_x, y: list_y + (row - ep_scroll) as u16, width: ep_w, height: 1 });
            }
            if rows_total > list_h {
                let more = format!("{}/{} рядків", (ep_scroll + list_h).min(rows_total), rows_total);
                f.render_widget(Line::styled(more, dim_s).right_aligned(), Rect { x: ep_x, y, width: ep_w, height: 1 });
            }
            new_ep_scroll = ep_scroll;
        }
        y = list_y + list_h as u16 + 1;

        // Players.
        let cur_src = self.current_source().map(|s| s.provider.clone());
        let page = self.page.as_ref().unwrap();
        let mut px = x;
        let label = Span::styled("Плеєр    ", if focus == Focus::Players { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new().add_modifier(Modifier::BOLD) });
        f.render_widget(label, Rect { x: px, y, width: 9, height: 1 });
        px += 9;
        if let Some((_, ep)) = self.current() {
            for s in &ep.sources {
                let sel = cur_src.as_deref() == Some(s.provider.as_str());
                let mpv_mark = if play::supports_mpv(&s.provider) { " ◆" } else { "" };
                let text = format!(" {}{mpv_mark} ", s.provider);
                let style = match (sel, focus == Focus::Players) {
                    (true, true) => Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD),
                    (true, false) => Style::new().bg(SELECTED).add_modifier(Modifier::BOLD),
                    _ => Style::new().fg(Color::Gray),
                };
                let tw = text.width() as u16;
                let rect = Rect { x: px, y, width: tw, height: 1 };
                f.render_widget(Span::styled(text, style), rect);
                hits.push((rect, Hit::Provider(s.provider.clone())));
                px += tw + 1;
            }
            f.render_widget(Span::styled("  ◆ — відкривається в mpv", dim_s), Rect { x: px, y, width: (x + w).saturating_sub(px), height: 1 });
        }
        y += 2;

        // Buttons.
        let mpv_ok = self.mpv_ready();
        let ep_label = self.current().map(|(_, e)| fmt_num(e.number.floor())).unwrap_or_default();
        let status_label = match &entry {
            Some(e) => format!("Список: {}", list_status_label(&e.status)),
            None => "До списку".into(),
        };
        let mut bx = x;
        for (i, b) in BUTTONS.iter().enumerate() {
            let text = match b {
                Btn::Mpv => " ▶ mpv ".to_string(),
                Btn::Browser => " ↗ Браузер ".to_string(),
                Btn::Watched => format!(" ✓ Переглянуто {ep_label} "),
                Btn::Status => format!(" {status_label} "),
                Btn::Site => " mikai.me ".to_string(),
            };
            let primary = (*b == Btn::Mpv && mpv_ok) || (*b == Btn::Browser && !mpv_ok);
            let disabled = *b == Btn::Mpv && !mpv_ok;
            let focused = focus == Focus::Buttons && page.btn == i;
            let style = if focused {
                Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD)
            } else if disabled {
                Style::new().bg(Color::Rgb(36, 34, 42)).fg(Color::DarkGray)
            } else if primary {
                Style::new().bg(SELECTED).add_modifier(Modifier::BOLD)
            } else {
                Style::new().bg(Color::Rgb(40, 38, 48))
            };
            let tw = text.width() as u16;
            let rect = Rect { x: bx, y, width: tw, height: 1 };
            f.render_widget(Span::styled(text, style), rect);
            hits.push((rect, Hit::Button(*b)));
            bx += tw + 2;
        }
        y += 2;

        // Result of the last action.
        if let Some(e) = page.last_cmd.and_then(|id| self.entries.iter().rev().find(|e| e.id == id)) {
            let mut out = vec![];
            if e.pending {
                out.push(Line::from(vec![
                    Span::styled(SPINNER[self.tick % SPINNER.len()], Style::new().fg(ACCENT)),
                    Span::styled(format!(" {}", e.cmd), dim_s),
                ]));
            } else if let Some(l) = e.lines.iter().find(|l| !l.segs.is_empty()) {
                wrap_line(&l.clone().nowrap(), w as usize, "", &mut out);
            }
            f.render_widget(Paragraph::new(out), Rect { x, y, width: w, height: 1 });
        }

        if let Some(p) = &mut self.page {
            p.hits = hits;
            p.cols = cols;
            p.ep_scroll = new_ep_scroll;
            p.rel_scroll = {
                let mut s = p.rel_scroll.min(p.rel);
                if p.rel >= s + list_h {
                    s = p.rel + 1 - list_h;
                }
                s
            };
        }
    }

    pub(super) fn page_hints(&self) -> &'static str {
        "enter ▶ · m mpv · b браузер · w переглянуто · s список · p плеєр · tab фокус · esc назад"
    }
}
