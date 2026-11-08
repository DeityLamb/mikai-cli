//! Full-screen watch page of one title.
//!
//! The page itself is about the episode: info, the chosen team and player,
//! the episode grid and the actions. Team (track first, then team) and player
//! are picked in modals. Key hints live inside the elements they drive. Every
//! action runs as a normal `/command`, so it also lands in the transcript.

use super::*;
use crate::play;
use super::ui::{self, FAINT, GREEN, MUTED, SELECT, SURFACE, YELLOW};

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

    fn modal_items(&self) -> Vec<ui::Item> {
        let Some((page, session, Some(pl))) = self.page_parts() else { return vec![] };
        let numbered = |i: usize, mut it: ui::Item| {
            it.key = (i < 9).then(|| (i + 1).to_string());
            it
        };
        match &page.modal {
            None => vec![],
            Some(Modal::Kind) => self
                .kinds()
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    let n = self.teams_of(k).len();
                    numbered(i, ui::Item::new(kind_title(k), format!("{n} {}", teams_word(n))))
                })
                .collect(),
            Some(Modal::Team(k)) => self
                .teams_of(k)
                .iter()
                .enumerate()
                .map(|(n, &i)| {
                    let r = &pl.releases[i];
                    let mut meta = vec![r.last_episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_else(|| "–".into())];
                    if let Some(d) = r.updated_at.as_deref().and_then(view::parse_time) {
                        meta.push(d.with_timezone(&Local).format("%d.%m.%y").to_string());
                    }
                    if page.mpv && playable(r) {
                        meta.push("mpv".into());
                    }
                    let mut it = ui::Item::new(r.team_names(), meta.join(" · "));
                    if session.team.as_deref() == Some(r.id.as_str()) {
                        it.tag = Some(("● дивились".into(), YELLOW));
                    }
                    numbered(n, it)
                })
                .collect(),
            Some(Modal::Player) => self
                .current()
                .map(|(_, e)| {
                    e.sources
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let meta = if play::supports_mpv(&s.provider) { "браузер і mpv" } else { "лише браузер" };
                            numbered(i, ui::Item::new(s.provider.clone(), meta))
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

    pub(super) fn page_hints(&self) -> Vec<(&'static str, &'static str)> {
        if self.page.as_ref().is_some_and(|p| p.modal.is_some()) {
            vec![]
        } else {
            vec![("←→↑↓", "серія"), ("Tab", "кнопки"), ("Ctrl+H", "нещодавні"), ("Ctrl+P", "меню"), ("Esc", "назад")]
        }
    }

    // ---------- drawing ----------

    pub(super) fn draw_page(&mut self, f: &mut Frame, area: Rect) {
        if self.page_parts().is_none() {
            self.page = None;
            return;
        }
        let mut hits: Vec<(Rect, Hit)> = vec![];
        // Panels span x-1..x+w+1 and keep one column free on the left for the focus bar.
        let x = area.x + 3;
        let w = area.width.saturating_sub(6);
        let bottom = area.y + area.height;
        let mut y = area.y + 1;

        y = self.draw_topbar(f, x, y, w, &mut hits);
        y = self.draw_info(f, x, y, w, bottom, &mut hits);

        let (page, _, player) = self.page_parts().unwrap();
        if page.detail.licensed || player.is_none() {
            let text = if page.detail.licensed {
                "Ліцензовано в Україні — дивіться легально"
            } else {
                "завантаження серій…"
            };
            let mut spans = vec![Span::styled(text.to_string(), Style::new().fg(if page.detail.licensed { YELLOW } else { MUTED }))];
            if page.detail.licensed {
                spans.push(Span::raw("   "));
                spans.extend(ui::hint("o", "mikai.me"));
                hits.push((Rect { x, y, width: w, height: 1 }, Hit::Button(Btn::Site)));
            }
            f.render_widget(Line::from(spans), Rect { x, y, width: w, height: 1 });
            self.page.as_mut().unwrap().hits = hits;
            return;
        }

        y = self.draw_choice(f, x, y, w, &mut hits);

        let buttons_h = 4;
        let (rows, cols) = self.episode_rows(w);
        let grid_h = rows.clamp(1, bottom.saturating_sub(y + 4 + buttons_h).max(1) as usize);
        let ep_scroll = self.draw_episodes(f, x, y, w, grid_h, cols, &mut hits);
        y += grid_h as u16 + 5;

        self.draw_buttons(f, x, y, &mut hits);
        y += 2;
        self.draw_result(f, x, y, w);

        if let Some(p) = &mut self.page {
            p.cols = cols;
            p.ep_scroll = ep_scroll;
        }
        if self.page.as_ref().is_some_and(|p| p.modal.is_some()) {
            hits = self.draw_modal(f, area);
        }
        if let Some(p) = &mut self.page {
            p.hits = hits;
        }
    }

    /// `[Esc] назад` on the left, list status on the right.
    fn draw_topbar(&self, f: &mut Frame, x: u16, y: u16, w: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        let (page, session, _) = self.page_parts().unwrap();
        let back = ui::hint("Esc", "назад");
        hits.push((Rect { x, y, width: ui::width(&back), height: 1 }, Hit::Back));
        f.render_widget(Line::from(back), Rect { x, y, width: w, height: 1 });
        let total = page.detail.episodes.total.filter(|&t| t > 0).map(|t| t.to_string()).unwrap_or_else(|| "?".into());
        let status = match session.entry.clone().flatten() {
            Some(e) => Span::styled(
                format!("● {} · {}/{total}", list_status_label(&e.status), e.episodes.map(fmt_num).unwrap_or_else(|| "0".into())),
                Style::new().fg(GREEN),
            ),
            None if self.client.has_key() => Span::styled("○ немає в списках", ui::muted()),
            None => Span::raw(""),
        };
        f.render_widget(Line::from(status).right_aligned(), Rect { x, y, width: w, height: 1 });
        y + 2
    }

    /// Info panel: title, meta, scores, genres, description.
    fn draw_info(&self, f: &mut Frame, x: u16, y: u16, w: u16, bottom: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        let (page, _, _) = self.page_parts().unwrap();
        let d = &page.detail;
        let mut lines: Vec<Line> = vec![Line::styled(d.titles.main().to_string(), ui::bold())];
        if let Some(sec) = d.titles.secondary() {
            lines.push(Line::styled(sec.to_string(), ui::muted()));
        }
        lines.push(Line::raw(""));
        let eps = match (d.episodes.localized, d.episodes.total) {
            (Some(l), Some(t)) => Some(format!("{l}/{} еп. українською", if t == 0 { "?".into() } else { t.to_string() })),
            _ => None,
        };
        let meta = [
            d.format.as_deref().map(str::to_uppercase),
            d.year.map(|y| y.to_string()),
            d.status.as_deref().map(|s| anime_status_label(s).to_string()),
            d.studio.clone(),
            d.age_rating.as_deref().map(str::to_uppercase),
            eps,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        lines.push(Line::raw(meta));
        let mut scores = vec![];
        for (name, v, prec) in [("Mikai", d.scores.mikai, 1), ("MAL", d.scores.mal, 2), ("Hikka", d.scores.hikka, 2)] {
            if let Some(v) = v {
                if !scores.is_empty() {
                    scores.push(Span::styled("  ·  ", ui::muted()));
                }
                scores.push(Span::styled(format!("{name} "), ui::muted()));
                scores.push(Span::styled(format!("{v:.prec$}"), ui::bold()));
            }
        }
        if !d.genres.is_empty() {
            if !scores.is_empty() {
                scores.push(Span::styled("  ·  ", ui::muted()));
            }
            scores.push(Span::styled(d.genres.iter().map(NamedUa::label).collect::<Vec<_>>().join(", "), ui::muted()));
        }
        lines.push(Line::from(scores));

        let mut desc_rows = 0;
        let mut more = "";
        if let Some(desc) = &d.description {
            let mut wrapped = vec![];
            for p in desc.trim().lines().filter(|l| !l.trim().is_empty()) {
                wrap_line(&SLine::of(p, Tone::Plain), w.saturating_sub(4) as usize, "", &mut wrapped);
            }
            let max = if page.full_desc { wrapped.len() } else { 2 };
            more = if wrapped.len() > max { "опис" } else if page.full_desc { "згорнути" } else { "" };
            lines.push(Line::raw(""));
            desc_rows = wrapped.len().min(max);
            lines.extend(wrapped.into_iter().take(max));
        }
        // Keep room for the rest of the page.
        let h = lines.len().min(bottom.saturating_sub(y + 18).max(4) as usize) as u16;
        ui::fill(f, Rect { x: x - 1, y, width: w + 2, height: h + 2 }, SURFACE);
        let inner = Rect { x: x + 1, y: y + 1, width: w.saturating_sub(2), height: h };
        f.render_widget(Paragraph::new(lines.into_iter().take(h as usize).collect::<Vec<_>>()), inner);
        if !more.is_empty() {
            // Like other panels: the key sits at the right of the header row.
            let chip = ui::hint("i", more);
            let cw = ui::width(&chip);
            let r = Rect { x: inner.x + inner.width - cw, y: inner.y, width: cw, height: 1 };
            f.render_widget(Line::from(chip), r);
            hits.push((r, Hit::Desc));
        }
        if desc_rows > 0 {
            hits.push((Rect { x: inner.x, y: inner.y + h - desc_rows as u16, width: inner.width, height: desc_rows as u16 }, Hit::Desc));
        }
        y + h + 3
    }

    /// Two panels: chosen team and chosen player, each opening its picker.
    fn draw_choice(&self, f: &mut Frame, x: u16, y: u16, w: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        let (page, _, Some(pl)) = self.page_parts().unwrap() else { return y };
        let gap = 4;
        let team_w = (w - gap) * 3 / 5;
        let player_x = x + team_w + gap;
        let player_w = w - team_w - gap;

        let team = pl.releases.get(page.rel);
        let team_value = Line::from(match team {
            Some(r) => vec![
                Span::styled(r.team_names(), ui::bold()),
                Span::styled(
                    format!("   {} · {} еп.", r.kind_label(), r.last_episode.map(fmt_num).unwrap_or_else(|| "?".into())),
                    ui::muted(),
                ),
            ],
            None => vec![Span::styled("—", ui::muted())],
        });
        let src = self.current_source();
        let player_value = Line::from(match src {
            Some(s) => vec![
                Span::styled(s.provider.clone(), ui::bold()),
                Span::styled(
                    if play::supports_mpv(&s.provider) { "   браузер і mpv" } else { "   лише браузер" },
                    ui::muted(),
                ),
            ],
            None => vec![Span::styled("—", ui::muted())],
        });

        for (px, pw, title, key, value, modal) in [
            (x, team_w, "Команда", "c", team_value, Modal::Kind),
            (player_x, player_w, "Плеєр", "p", player_value, Modal::Player),
        ] {
            let panel = Rect { x: px - 1, y, width: pw + 2, height: 4 };
            ui::fill(f, panel, SURFACE);
            ui::section(f, Rect { x: px + 1, y: y + 1, width: pw - 2, height: 1 }, title, "", vec![ui::key(key)], false);
            f.render_widget(value, Rect { x: px + 1, y: y + 2, width: pw - 2, height: 1 });
            hits.push((panel, Hit::Open(modal)));
        }
        y + 5
    }

    /// Rows the grid needs and its column count at width `w`.
    fn episode_rows(&self, w: u16) -> (usize, usize) {
        let Some((p, _, Some(pl))) = self.page_parts() else { return (1, 1) };
        pl.releases.get(p.rel).map_or((1, 1), |rel| grid_shape(rel, w.saturating_sub(2)))
    }

    /// Episode panel. Returns the scroll offset it used.
    #[allow(clippy::too_many_arguments)]
    fn draw_episodes(&self, f: &mut Frame, x: u16, y: u16, w: u16, h: usize, cols: usize, hits: &mut Vec<(Rect, Hit)>) -> usize {
        let (page, session, Some(pl)) = self.page_parts().unwrap() else { return 0 };
        let Some(rel) = pl.releases.get(page.rel) else { return 0 };
        let on = page.focus == Focus::Episodes;
        let panel_h = h as u16 + 4;
        ui::fill(f, Rect { x: x - 1, y, width: w + 2, height: panel_h }, SURFACE);
        if on {
            ui::focus_bar(f, x - 2, y, panel_h);
        }
        let ix = x + 1;
        let iw = w.saturating_sub(2);
        ui::section(
            f,
            Rect { x: ix, y: y + 1, width: iw, height: 1 },
            "Серії",
            "",
            vec![Span::styled(format!("{} / {}", page.ep + 1, rel.episodes.len()), ui::muted())],
            on,
        );
        let watched = watched_of(session);
        let cell_w = cell_width(rel);
        let scroll = follow(page.ep_scroll, page.ep / cols, h);
        for row in scroll..(scroll + h) {
            let ry = y + 3 + (row - scroll) as u16;
            for col in 0..cols {
                let i = row * cols + col;
                let Some(e) = rel.episodes.get(i) else { break };
                let seen = e.number <= watched;
                let opened = session.last_ep == Some(e.number);
                let mark = if seen { "✓" } else if opened { "▸" } else { " " };
                let mut style = if seen {
                    Style::new().fg(GREEN)
                } else if opened {
                    Style::new().fg(YELLOW)
                } else {
                    Style::new()
                };
                if i == page.ep {
                    style = style.bg(SELECT).add_modifier(Modifier::BOLD);
                }
                let rect = Rect { x: ix + (col * cell_w) as u16, y: ry, width: cell_w as u16 - 1, height: 1 };
                f.render_widget(Span::styled(format!(" {mark} {}", crate::style::pad(&e.label(), cell_w - 4)), style), rect);
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
        let on = page.focus == Focus::Buttons;
        if on {
            ui::focus_bar(f, x - 2, y, 1);
        }
        let mut bx = x;
        for (i, b) in BUTTONS.iter().enumerate() {
            let primary = (*b == Btn::Mpv && mpv_ok) || (*b == Btn::Browser && !mpv_ok);
            let (k, label) = match b {
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
            let focused = on && page.btn == i;
            let label_style = if focused {
                Style::new().bg(SELECT).add_modifier(Modifier::BOLD)
            } else if disabled {
                Style::new().bg(ui::KEY_BG).fg(FAINT)
            } else {
                Style::new().bg(ui::KEY_BG)
            };
            let spans = vec![
                if primary { ui::key_primary(k) } else { ui::key(k) },
                Span::styled(format!(" {label} "), label_style),
            ];
            let tw = ui::width(&spans);
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
                Span::styled(format!(" {}", e.cmd), ui::muted()),
            ]));
        } else if let Some(l) = e.lines.iter().find(|l| !l.segs.is_empty()) {
            wrap_line(&l.clone().nowrap(), w as usize, "", &mut out);
        }
        f.render_widget(Paragraph::new(out), Rect { x, y, width: w, height: 1 });
    }

    /// Team or player picker over the dimmed page. Returns the click map.
    fn draw_modal(&self, f: &mut Frame, area: Rect) -> Vec<(Rect, Hit)> {
        let Some(page) = &self.page else { return vec![] };
        let Some(m) = &page.modal else { return vec![] };
        f.buffer_mut().set_style(area, Style::new().fg(FAINT));
        let two_steps = self.kinds().len() > 1;
        let (title, aside) = match m {
            Modal::Kind => ("Трек".to_string(), if two_steps { "крок 1/2" } else { "" }),
            Modal::Team(k) => (format!("Команда · {}", kind_title(k)), if two_steps { "крок 2/2" } else { "" }),
            Modal::Player => (format!("Плеєр · {} еп.", self.current().map(|(_, e)| e.label()).unwrap_or_default()), ""),
        };
        let mut footer = vec![("↑↓", "вибір"), ("⏎", "обрати")];
        if matches!(m, Modal::Team(_)) && two_steps {
            footer.push(("←", "трек"));
        }
        footer.push(("Esc", "закрити"));
        let overlay = ui::Overlay {
            title,
            aside: aside.into(),
            filter: None,
            items: self.modal_items(),
            sel: page.modal_sel,
            empty: "нічого немає".into(),
            footer,
            width: 64,
        };
        let layout = overlay.draw(f, area);
        let mut hits = vec![(area, Hit::Outside), (layout.rect, Hit::Inside)];
        hits.extend(layout.rows.into_iter().map(|(r, i)| (r, Hit::Item(i))));
        hits
    }
}

fn cell_width(rel: &Release) -> usize {
    rel.episodes.iter().map(|e| e.label().width()).max().unwrap_or(1) + 5
}

fn grid_shape(rel: &Release, w: u16) -> (usize, usize) {
    let cols = (w as usize / cell_width(rel)).max(1);
    (rel.episodes.len().div_ceil(cols).max(1), cols)
}
