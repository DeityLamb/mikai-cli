//! Full-screen watch page of one title.
//!
//! The page itself is about the episode: info, the chosen team and player,
//! the episode grid and the actions. Team (track first, then team) and player
//! are picked in modals. Key hints live inside the elements they drive. Every
//! action runs as a normal `/command`, so it also lands in the transcript.

use super::*;
use crate::play;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Episodes,
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

/// Pickers shown over the page.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Modal {
    Kind,
    Team(String),
    Player,
}

#[derive(Clone)]
pub(super) enum Hit {
    Back,
    Desc,
    Open(Modal),
    Episode(usize),
    Button(Btn),
    /// Row `n` of the open modal.
    Item(usize),
    /// Anywhere on the dimmed page while a modal is open.
    Outside,
    /// The modal panel itself, between rows.
    Inside,
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
    ep_scroll: usize,
    modal: Option<Modal>,
    modal_sel: usize,
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
            ep_scroll: 0,
            modal: None,
            modal_sel: 0,
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
        .max_by_key(|(i, r)| (mpv && playable(r), r.episodes.len(), std::cmp::Reverse(*i)))
        .map(|(i, _)| i)
}

fn playable(r: &Release) -> bool {
    r.episodes.iter().any(|e| e.sources.iter().any(|s| play::supports_mpv(&s.provider)))
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

fn teams_word(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (1, r) if r != 11 => "команда",
        (2..=4, r) if !(12..=14).contains(&r) => "команди",
        _ => "команд",
    }
}

/// Clamps `scroll` so that `sel` stays inside a window of `h` rows.
fn follow(scroll: usize, sel: usize, h: usize) -> usize {
    let s = scroll.min(sel);
    if h > 0 && sel >= s + h { sel + 1 - h } else { s }
}

/// One row of a modal.
struct MItem {
    label: String,
    hint: String,
    accent: String,
    disabled: bool,
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
                    self.page_note("цей плеєр не віддає пряме відео — оберіть плеєр з ◆ (p) або браузер (b)");
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

    // ---------- modals ----------

    fn kinds(&self) -> Vec<String> {
        let Some((_, _, Some(pl))) = self.page_parts() else { return vec![] };
        ["voice", "sub"].iter().filter(|k| pl.releases.iter().any(|r| r.kind == **k)).map(|k| k.to_string()).collect()
    }

    /// Release indices of `kind`.
    fn teams_of(&self, kind: &str) -> Vec<usize> {
        let Some((_, _, Some(pl))) = self.page_parts() else { return vec![] };
        pl.releases.iter().enumerate().filter(|(_, r)| r.kind == kind).map(|(i, _)| i).collect()
    }

    fn open_modal(&mut self, m: Modal) {
        let Some((page, _, Some(pl))) = self.page_parts() else { return };
        let (m, sel) = match m {
            // With a single track, go straight to its teams.
            Modal::Kind if self.kinds().len() == 1 => {
                let k = self.kinds().remove(0);
                let sel = self.teams_of(&k).iter().position(|&i| i == page.rel).unwrap_or(0);
                (Modal::Team(k), sel)
            }
            Modal::Kind => (Modal::Kind, self.kinds().iter().position(|k| *k == page.kind).unwrap_or(0)),
            Modal::Team(k) => {
                let teams = self.teams_of(&k);
                let sel = teams
                    .iter()
                    .position(|&i| i == page.rel)
                    .or_else(|| best_release(pl, Some(&k), page.mpv).and_then(|b| teams.iter().position(|&i| i == b)))
                    .unwrap_or(0);
                (Modal::Team(k), sel)
            }
            Modal::Player => {
                let cur = self.current_source().map(|s| s.provider.clone());
                let sel = self
                    .current()
                    .and_then(|(_, e)| e.sources.iter().position(|s| Some(&s.provider) == cur.as_ref()))
                    .unwrap_or(0);
                (Modal::Player, sel)
            }
        };
        if let Some(p) = &mut self.page {
            p.modal = Some(m);
            p.modal_sel = sel;
        }
    }

    fn modal_items(&self) -> Vec<MItem> {
        let Some((page, session, Some(pl))) = self.page_parts() else { return vec![] };
        match &page.modal {
            None => vec![],
            Some(Modal::Kind) => self
                .kinds()
                .iter()
                .map(|k| {
                    let n = self.teams_of(k).len();
                    MItem { label: kind_title(k).into(), hint: format!("{n} {}", teams_word(n)), accent: String::new(), disabled: false }
                })
                .collect(),
            Some(Modal::Team(k)) => self
                .teams_of(k)
                .iter()
                .map(|&i| {
                    let r = &pl.releases[i];
                    let eps = r.last_episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_else(|| "–".into());
                    let upd = r.updated_at.as_deref().and_then(view::parse_time).map(view::fmt_date).unwrap_or_default();
                    let mine = session.team.as_deref() == Some(r.id.as_str());
                    MItem {
                        label: format!("{}{}", r.team_names(), if mine { "  ·  ви дивились" } else { "" }),
                        hint: format!("{eps:>8}   {upd}"),
                        accent: if page.mpv && playable(r) { "◆".into() } else { String::new() },
                        disabled: false,
                    }
                })
                .collect(),
            Some(Modal::Player) => self
                .current()
                .map(|(_, e)| {
                    e.sources
                        .iter()
                        .map(|s| {
                            let mpv = play::supports_mpv(&s.provider);
                            MItem {
                                label: s.provider.clone(),
                                hint: if mpv { "браузер або mpv".into() } else { "лише браузер".into() },
                                accent: if mpv { "◆ mpv".into() } else { String::new() },
                                disabled: false,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn modal_pick(&mut self, i: usize) {
        let Some(m) = self.page.as_ref().and_then(|p| p.modal.clone()) else { return };
        match m {
            Modal::Kind => {
                if let Some(k) = self.kinds().get(i).cloned() {
                    self.open_modal(Modal::Team(k));
                }
            }
            Modal::Team(k) => {
                if let Some(&rel) = self.teams_of(&k).get(i) {
                    self.select_release(rel);
                }
                self.close_modal();
            }
            Modal::Player => {
                let name = self.current().and_then(|(_, e)| e.sources.get(i)).map(|s| s.provider.clone());
                if let (Some(p), Some(name)) = (&mut self.page, name) {
                    p.provider = Some(name);
                }
                self.close_modal();
            }
        }
    }

    fn close_modal(&mut self) {
        if let Some(p) = &mut self.page {
            p.modal = None;
        }
    }

    fn modal_key(&mut self, k: KeyEvent) {
        let n = self.modal_items().len();
        let Some(p) = &mut self.page else { return };
        match k.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                // From teams, step back to the track choice if there is one.
                let back = matches!(p.modal, Some(Modal::Team(_))) && k.code != KeyCode::Esc;
                if back && self.kinds().len() > 1 {
                    self.open_modal(Modal::Kind);
                } else {
                    self.close_modal();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => p.modal_sel = p.modal_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => p.modal_sel = (p.modal_sel + 1).min(n.saturating_sub(1)),
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('l') => {
                let i = p.modal_sel;
                self.modal_pick(i);
            }
            KeyCode::Char(c @ '1'..='9') => {
                let i = c as usize - '1' as usize;
                if i < n {
                    self.modal_pick(i);
                }
            }
            KeyCode::Char('c') => self.open_modal(Modal::Kind),
            KeyCode::Char('p') => self.open_modal(Modal::Player),
            _ => {}
        }
    }

    // ---------- keys ----------

    pub(super) fn page_key(&mut self, k: KeyEvent) {
        if self.page_parts().is_none() {
            self.page = None;
            return;
        }
        if self.page.as_ref().is_some_and(|p| p.modal.is_some()) {
            self.modal_key(k);
            return;
        }
        let ep_len = self.current().map(|(r, _)| r.episodes.len()).unwrap_or(0);
        let Some(p) = &mut self.page else { return };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.page = None,
            KeyCode::Tab | KeyCode::BackTab => {
                p.focus = if p.focus == Focus::Episodes { Focus::Buttons } else { Focus::Episodes };
            }
            KeyCode::Char('c') => self.open_modal(Modal::Kind),
            KeyCode::Char('p') => self.open_modal(Modal::Player),
            KeyCode::Char('m') => self.page_action(Btn::Mpv),
            KeyCode::Char('b') => self.page_action(Btn::Browser),
            KeyCode::Char('w') => self.page_action(Btn::Watched),
            KeyCode::Char('s') => self.page_action(Btn::Status),
            KeyCode::Char('o') => self.page_action(Btn::Site),
            KeyCode::Char('i') => p.full_desc = !p.full_desc,
            KeyCode::Enter | KeyCode::Char(' ') => match p.focus {
                Focus::Buttons => {
                    let b = BUTTONS[p.btn];
                    self.page_action(b);
                }
                Focus::Episodes => self.primary(),
            },
            code => match p.focus {
                Focus::Episodes => {
                    let cols = p.cols.max(1);
                    let last = ep_len.saturating_sub(1);
                    match code {
                        KeyCode::Left | KeyCode::Char('h') => p.ep = p.ep.saturating_sub(1),
                        KeyCode::Right | KeyCode::Char('l') => p.ep = (p.ep + 1).min(last),
                        KeyCode::Up | KeyCode::Char('k') => p.ep = p.ep.saturating_sub(cols),
                        KeyCode::Down | KeyCode::Char('j') if p.ep / cols == last / cols => p.focus = Focus::Buttons,
                        KeyCode::Down | KeyCode::Char('j') => p.ep = (p.ep + cols).min(last),
                        KeyCode::Home | KeyCode::Char('g') => p.ep = 0,
                        KeyCode::End | KeyCode::Char('G') => p.ep = last,
                        _ => {}
                    }
                }
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
        // Topmost hit wins: modal rows are pushed last.
        let Some(hit) = p.hits.iter().rev().find(|(r, _)| r.contains((col, row).into())).map(|(_, h)| h.clone()) else { return };
        match hit {
            Hit::Back => self.page = None,
            Hit::Outside => self.close_modal(),
            Hit::Inside => {}
            Hit::Item(i) => self.modal_pick(i),
            Hit::Open(m) => self.open_modal(m),
            Hit::Desc => {
                if let Some(p) = &mut self.page {
                    p.full_desc = !p.full_desc;
                }
            }
            Hit::Episode(i) => {
                // A second click on the selected episode plays it.
                let again = self.page.as_ref().is_some_and(|p| p.ep == i && p.focus == Focus::Episodes);
                if let Some(p) = &mut self.page {
                    p.ep = i;
                    p.focus = Focus::Episodes;
                }
                if again {
                    self.primary();
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

    pub(super) fn page_hints(&self) -> &'static str {
        if self.page.as_ref().is_some_and(|p| p.modal.is_some()) {
            "↑↓ вибір · ⏎ обрати · ← назад · esc закрити"
        } else {
            "←→↑↓ серія · tab кнопки · ctrl+h нещодавні · ctrl+p меню · esc назад"
        }
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

        // Team and player, each on its own panel.
        self.draw_choice(f, x, y, w, &mut hits);
        y += 5;

        // Episodes.
        let buttons_h = 4;
        let avail = bottom.saturating_sub(y + 3 + buttons_h).max(2) as usize;
        let focus_eps = self.page.as_ref().is_some_and(|p| p.focus == Focus::Episodes);
        let (rows, cols) = self.episode_rows(w);
        let grid_h = rows.clamp(1, avail);
        fill(f, Rect { x: x - 1, y: y - 1, width: w + 2, height: grid_h as u16 + 4 }, if focus_eps { PANEL_FOCUS } else { PANEL_BG });
        let ep_scroll = self.draw_episodes(f, x, y, w, grid_h, cols, &mut hits);
        y += grid_h as u16 + 4;

        self.draw_buttons(f, x, y, &mut hits);
        y += 2;
        self.draw_result(f, x, y, w);

        if let Some(p) = &mut self.page {
            p.cols = cols;
            p.ep_scroll = ep_scroll;
        }
        if self.page.as_ref().is_some_and(|p| p.modal.is_some()) {
            // Clicks outside the modal close it.
            hits.clear();
            hits.push((area, Hit::Outside));
            self.draw_modal(f, area, &mut hits);
        }
        if let Some(p) = &mut self.page {
            p.hits = hits;
        }
    }

    /// Title, meta and description on a panel. Returns the next free row.
    fn draw_info(&self, f: &mut Frame, x: u16, y: u16, w: u16, bottom: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        let (page, session, _) = self.page_parts().unwrap();
        let d = &page.detail;
        let indent = "       ";
        let mut row = y;

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
        f.render_widget(Line::from(title), Rect { x, y: row, width: w, height: 1 });
        hits.push((Rect { x, y: row, width: 6, height: 1 }, Hit::Back));
        row += 1;

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
            let hint = if lines.len() > max { "ще" } else if page.full_desc { "згорнути" } else { "" };
            rendered.push(Line::raw(""));
            desc_rows = lines.len().min(max);
            rendered.extend(lines.into_iter().take(max));
            if !hint.is_empty() {
                rendered.push(Line::from(vec![Span::raw(indent), key_span("i"), Span::styled(format!(" {hint}"), dim())]));
                desc_rows += 1;
            }
        }
        // Leave room for the rest of the page.
        let max_h = bottom.saturating_sub(row + 14) as usize;
        let h = rendered.len().min(max_h.max(3));
        f.render_widget(Paragraph::new(rendered.into_iter().take(h).collect::<Vec<_>>()), Rect { x, y: row, width: w, height: h as u16 });
        if desc_rows > 0 {
            hits.push((Rect { x, y: row + desc_top as u16, width: w, height: desc_rows as u16 }, Hit::Desc));
        }
        let end = row + h as u16;
        f.buffer_mut().set_style(Rect { x: x - 1, y: y - 1, width: w + 2, height: end - y + 2 }, Style::new().bg(PANEL_BG));
        end + 3
    }

    /// Two panels: the chosen team and the chosen player, each under its key chip.
    fn draw_choice(&self, f: &mut Frame, x: u16, y: u16, w: u16, hits: &mut Vec<(Rect, Hit)>) {
        let (page, _, Some(pl)) = self.page_parts().unwrap() else { return };
        let gap = 4;
        let team_w = (w - gap) * 3 / 5;
        let player_x = x + team_w + gap;
        let player_w = w - team_w - gap;
        for (px, pw, m) in [(x, team_w, Modal::Kind), (player_x, player_w, Modal::Player)] {
            let panel = Rect { x: px - 1, y: y - 1, width: pw + 2, height: 4 };
            fill(f, panel, PANEL_BG);
            hits.push((panel, Hit::Open(m)));
        }

        f.render_widget(Line::from(chip("c", "Команда", false, false)), Rect { x, y, width: team_w, height: 1 });
        if let Some(r) = pl.releases.get(page.rel) {
            let line = Line::from(vec![
                Span::styled(r.team_names(), Style::new().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("  {} · {} еп.", r.kind_label(), r.last_episode.map(fmt_num).unwrap_or_else(|| "?".into())),
                    Style::new().fg(Color::Gray),
                ),
            ]);
            f.render_widget(line, Rect { x: x + 1, y: y + 1, width: team_w.saturating_sub(1), height: 1 });
        }

        f.render_widget(Line::from(chip("p", "Плеєр", false, false)), Rect { x: player_x, y, width: player_w, height: 1 });
        if let Some(s) = self.current_source() {
            let mut spans = vec![Span::styled(s.provider.clone(), Style::new().add_modifier(Modifier::BOLD))];
            if play::supports_mpv(&s.provider) {
                spans.push(Span::styled("  ◆ mpv", Style::new().fg(ACCENT)));
            } else {
                spans.push(Span::styled("  лише браузер", dim()));
            }
            f.render_widget(Line::from(spans), Rect { x: player_x + 1, y: y + 1, width: player_w.saturating_sub(1), height: 1 });
        }
    }

    /// Rows the grid needs and its column count at width `w`.
    fn episode_rows(&self, w: u16) -> (usize, usize) {
        let Some((p, _, Some(pl))) = self.page_parts() else { return (1, 1) };
        pl.releases.get(p.rel).map_or((1, 1), |rel| grid_shape(rel, w))
    }

    /// Returns the scroll offset it used.
    #[allow(clippy::too_many_arguments)]
    fn draw_episodes(&self, f: &mut Frame, x: u16, y: u16, w: u16, h: usize, cols: usize, hits: &mut Vec<(Rect, Hit)>) -> usize {
        let (page, session, Some(pl)) = self.page_parts().unwrap() else { return 0 };
        let Some(rel) = pl.releases.get(page.rel) else { return 0 };
        let on = page.focus == Focus::Episodes;
        let title = Style::new().add_modifier(Modifier::BOLD).fg(if on { ACCENT } else { Color::Reset });
        f.render_widget(
            Line::from(vec![
                Span::styled("Серії", title),
                Span::styled(format!("   {} · {}", rel.team_names(), rel.kind_label()), dim()),
            ]),
            Rect { x, y, width: w, height: 1 },
        );
        f.render_widget(
            Line::styled(format!("{} / {}", page.ep + 1, rel.episodes.len()), dim()).right_aligned(),
            Rect { x, y, width: w, height: 1 },
        );
        let watched = watched_of(session);
        let cell_w = cell_width(rel);
        let scroll = follow(page.ep_scroll, page.ep / cols, h);
        for row in scroll..(scroll + h) {
            let ry = y + 2 + (row - scroll) as u16;
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
                    style = style.bg(if on { SELECTED } else { Color::Rgb(56, 48, 64) }).add_modifier(Modifier::BOLD);
                }
                let rect = Rect { x: x + (col * cell_w) as u16, y: ry, width: cell_w as u16 - 1, height: 1 };
                f.render_widget(Span::styled(format!("{mark} {} ", crate::style::pad(&e.label(), cell_w - 4)), style), rect);
                hits.push((rect, Hit::Episode(i)));
            }
        }
        scroll
    }

    fn draw_buttons(&self, f: &mut Frame, x: u16, y: u16, hits: &mut Vec<(Rect, Hit)>) {
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

    fn draw_modal(&self, f: &mut Frame, area: Rect, hits: &mut Vec<(Rect, Hit)>) {
        let Some(page) = &self.page else { return };
        let Some(m) = &page.modal else { return };
        let items = self.modal_items();
        // Dim the page behind.
        f.buffer_mut().set_style(area, Style::new().fg(Color::Rgb(70, 68, 78)));

        let (title, step) = match m {
            Modal::Kind => ("Трек".to_string(), if self.kinds().len() > 1 { "1/2" } else { "" }),
            Modal::Team(k) => (format!("{} — команда", kind_title(k)), if self.kinds().len() > 1 { "2/2" } else { "" }),
            Modal::Player => (
                format!("Плеєр · {} еп.", self.current().map(|(_, e)| e.label()).unwrap_or_default()),
                "",
            ),
        };
        let w = 60.min(area.width.saturating_sub(4));
        let list_h = items.len().min(area.height.saturating_sub(10) as usize).max(1);
        let h = list_h as u16 + 6;
        let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + area.height.saturating_sub(h) / 3, width: w, height: h };
        f.render_widget(Clear, rect);
        f.render_widget(Block::new().style(Style::new().bg(PANEL)), rect);
        let ix = rect.x + 2;
        let iw = rect.width.saturating_sub(4);
        f.render_widget(
            Line::from(vec![
                Span::styled(title, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(if step.is_empty() { String::new() } else { format!("   крок {step}") }, dim()),
            ]),
            Rect { x: ix, y: rect.y + 1, width: iw, height: 1 },
        );
        let scroll = follow(0, page.modal_sel, list_h);
        for (row, (i, it)) in items.iter().enumerate().skip(scroll).take(list_h).enumerate() {
            let sel = i == page.modal_sel;
            let base = if sel { Style::new().bg(SELECTED) } else { Style::new() };
            let key = if i < 9 { format!(" {} ", i + 1) } else { "   ".into() };
            let accent_w = 6;
            let hint_w = items.iter().map(|x| x.hint.width()).max().unwrap_or(0).min(iw as usize / 2);
            let label_w = (iw as usize).saturating_sub(3 + 1 + hint_w + 2 + accent_w);
            let ry = rect.y + 3 + row as u16;
            let line = Line::from(vec![
                Span::styled(key, base.fg(ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {}", crate::style::pad(&it.label, label_w)), base.add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() }).fg(if it.disabled { Color::DarkGray } else { Color::Reset })),
                Span::styled(format!("{}  ", crate::style::pad(&it.hint, hint_w)), base.fg(Color::Gray)),
                Span::styled(crate::style::pad(&it.accent, accent_w), base.fg(ACCENT)),
            ]);
            let r = Rect { x: ix, y: ry, width: iw, height: 1 };
            f.render_widget(line, r);
            hits.push((r, Hit::Item(i)));
        }
        let foot = match m {
            Modal::Team(_) if self.kinds().len() > 1 => "◆ — є плеєр для mpv · ← трек",
            Modal::Team(_) => "◆ — є плеєр для mpv",
            Modal::Player => "◆ — відкривається в mpv",
            Modal::Kind => "",
        };
        f.render_widget(Line::styled(foot, dim()), Rect { x: ix, y: rect.y + h - 2, width: iw, height: 1 });
        // Clicks inside the panel but not on a row do nothing.
        hits.insert(1, (rect, Hit::Inside));
    }
}

fn cell_width(rel: &Release) -> usize {
    rel.episodes.iter().map(|e| e.label().width()).max().unwrap_or(1) + 4
}

fn grid_shape(rel: &Release, w: u16) -> (usize, usize) {
    let cols = (w as usize / cell_width(rel)).max(1);
    (rel.episodes.len().div_ceil(cols).max(1), cols)
}

const PANEL_BG: Color = Color::Rgb(26, 25, 32);
const PANEL_FOCUS: Color = Color::Rgb(36, 32, 44);
const CHIP_BG: Color = Color::Rgb(44, 41, 53);

fn fill(f: &mut Frame, rect: Rect, bg: Color) {
    let area = f.area();
    f.buffer_mut().set_style(rect.intersection(area), Style::new().bg(bg));
}

/// A key-labelled chip like ` c  Команда `: the key on its own tint.
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
