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
    /// Track | team | player in three columns; the number is the active column.
    Source(usize),
    Related,
}

#[derive(Clone)]
pub(super) enum Hit {
    Back,
    Desc,
    Open(Modal),
    Episode(usize),
    Button(Btn),
    /// Open a related title.
    Related(String),
    /// Row `n` of the open list modal.
    Item(usize),
    /// Row `n` of column `col` in the source modal.
    Cell(usize, usize),
    /// Anywhere on the dimmed page while a modal is open.
    Outside,
    /// The modal panel itself, between rows.
    Inside,
}

pub(super) struct Page {
    /// The title's state: player, list entry, last team and episode.
    pub s: Session,
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
    pub fn new(detail: AnimeDetail, session: Session) -> Self {
        let mut page = Self {
            s: session,
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
        if let Some(p) = &page.s.player {
            let remembered = page.s.team.as_ref().and_then(|t| p.releases.iter().position(|r| &r.id == t));
            let rel = remembered.or_else(|| best_release(p, None, page.mpv)).unwrap_or(0);
            let kind = p.releases.get(rel).map(|r| r.kind.clone());
            let ep = next_episode(p.releases.get(rel), &page.s);
            page.rel = rel;
            page.ep = ep;
            if let Some(k) = kind {
                page.kind = k;
            }
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
    pub(super) fn open_page(&mut self, detail: AnimeDetail, session: Session) {
        self.page = Some(Page::new(detail, session));
    }

    /// The page with its title state and player.
    fn page_parts(&self) -> Option<(&Page, &Session, Option<&Player>)> {
        let p = self.page.as_ref()?;
        Some((p, &p.s, p.s.player.as_ref()))
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

    fn set_last(&mut self, id: u64) {
        if let Some(p) = &mut self.page {
            p.last_cmd = Some(id);
        }
    }

    /// A local message in the result line, also kept in the transcript.
    fn page_note(&mut self, text: &str) {
        self.note(vec![SLine::of(text, Tone::Yellow)]);
        if let Some(id) = self.entries.last().map(|e| e.id) {
            self.set_last(id);
        }
    }

    fn page_action(&mut self, b: Btn) {
        let Some((_, s, _)) = self.page_parts() else { return };
        let (r, title, url, entry) = (s.r.clone(), s.title.clone(), s.url.clone(), s.entry.clone().flatten());
        let Some((rel, ep)) = self.current() else { return };
        let (rel, ep) = (rel.clone(), ep.clone());
        let src = self.current_source().cloned();
        let id = match b {
            Btn::Mpv | Btn::Browser => {
                let mpv = b == Btn::Mpv;
                if mpv && !self.page.as_ref().is_some_and(|p| p.mpv) {
                    return self.page_note("mpv не встановлено");
                }
                if mpv && !self.mpv_ready() {
                    return self.page_note("цей плеєр не віддає пряме відео — оберіть інший плеєр (p) або браузер (b)");
                }
                let Some(src) = src else { return };
                self.run_action(move |_| {
                    cli::play(&cli::media_title(&title, &ep, &rel), &src, mpv)?;
                    let line = SLine::of("▶ ", Tone::Accent)
                        .push(title.clone(), Tone::Bold)
                        .push(format!("  ·  {} еп.  ·  {}  ·  {}", ep.label(), rel.team_names(), if mpv { "mpv".to_string() } else { src.provider.clone() }), Tone::Dim);
                    let played = Recent { r, title, ep: ep.number, team: Some(rel.id.clone()), team_name: Some(rel.team_names()), at: Utc::now() };
                    Ok(Outcome { lines: vec![line], played: Some(played), ..Default::default() })
                })
            }
            Btn::Watched | Btn::Status => {
                if !self.client.has_key() {
                    return self.page_note("щоб вести списки, увійдіть: /login");
                }
                let patch = if b == Btn::Watched {
                    let mut p = json!({ "episodes": ep.number.floor() as i64 });
                    if entry.is_none() {
                        p["status"] = json!("watching");
                    }
                    p
                } else {
                    let next = match entry.as_ref().and_then(|e| LIST_STATUSES.iter().position(|s| *s == e.status)) {
                        Some(i) => LIST_STATUSES[(i + 1) % LIST_STATUSES.len()],
                        None => "watching",
                    };
                    json!({ "status": next })
                };
                self.run_action(move |c| {
                    let e = c.save_entry(&r, &patch)?;
                    let what = if b == Btn::Watched {
                        format!("переглянуто до {}", e.episodes.map(fmt_num).unwrap_or_else(|| "0".into()))
                    } else {
                        format!("список: {}", list_status_label(&e.status))
                    };
                    let line = SLine::of("✓ ", Tone::Green).push(title, Tone::Bold).push(format!("  ·  {what}"), Tone::Dim);
                    Ok(Outcome { lines: vec![line], entry: Some((r, Some(e))), ..Default::default() })
                })
            }
            Btn::Site => {
                let Some(url) = url else { return };
                self.run_action(move |_| {
                    open::that_detached(&url).context("не вдалося відкрити браузер")?;
                    Ok(Outcome { lines: vec![SLine::of("↗ ", Tone::Accent).push(title, Tone::Bold).push("  ·  mikai.me", Tone::Dim)], ..Default::default() })
                })
            }
        };
        self.set_last(id);
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

    /// Related titles, seasons first.
    fn relations(&self) -> Vec<&Relation> {
        let Some(p) = &self.page else { return vec![] };
        let mut v: Vec<&Relation> = p.detail.relations.iter().filter(|r| r.anime.reference().is_some()).collect();
        v.sort_by_key(|r| r.rank());
        v
    }

    fn open_related(&mut self, r: String) {
        self.submit(format!("/view {r}"));
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

    /// Players the chosen team uses, in order of first appearance.
    fn players(&self) -> Vec<String> {
        let Some((p, _, Some(pl))) = self.page_parts() else { return vec![] };
        let mut out: Vec<String> = vec![];
        for e in pl.releases.get(p.rel).map(|r| r.episodes.as_slice()).unwrap_or(&[]) {
            for s in &e.sources {
                if !out.contains(&s.provider) {
                    out.push(s.provider.clone());
                }
            }
        }
        out
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

    fn open_modal(&mut self, m: Modal) {
        if let Some(p) = &mut self.page {
            p.modal = Some(m);
            p.modal_sel = 0;
        }
    }

    fn close_modal(&mut self) {
        if let Some(p) = &mut self.page {
            p.modal = None;
        }
    }

    /// Rows of the track, team and player columns, with the chosen row of each.
    fn source_columns(&self) -> [(Vec<ui::Item>, usize); 3] {
        let empty = || (vec![], 0);
        let Some((page, session, Some(pl))) = self.page_parts() else { return [empty(), empty(), empty()] };
        let kinds = self.kinds();
        let tracks = kinds
            .iter()
            .map(|k| {
                let n = self.teams_of(k).len();
                ui::Item::new(kind_title(k), format!("{n} {}", teams_word(n)))
            })
            .collect();
        let track_sel = kinds.iter().position(|k| *k == page.kind).unwrap_or(0);

        let ids = self.teams_of(&page.kind);
        let teams = ids
            .iter()
            .map(|&i| {
                let r = &pl.releases[i];
                let mut meta = r.last_episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_else(|| "–".into());
                if page.mpv && playable(r) {
                    meta.push_str(" · mpv");
                }
                let mut it = ui::Item::new(r.team_names(), meta);
                if session.team.as_deref() == Some(r.id.as_str()) {
                    it.tag = Some(("●".into(), YELLOW));
                }
                it
            })
            .collect();
        let team_sel = ids.iter().position(|&i| i == page.rel).unwrap_or(0);

        let names = self.players();
        let cur = self.current_source().map(|s| s.provider.clone());
        let players = names
            .iter()
            .map(|n| ui::Item::new(n.clone(), if play::supports_mpv(n) { "mpv" } else { "" }))
            .collect();
        let player_sel = names.iter().position(|n| Some(n) == cur.as_ref()).unwrap_or(0);
        [(tracks, track_sel), (teams, team_sel), (players, player_sel)]
    }

    /// Most rows any column of the source modal can have for this title.
    fn source_rows_max(&self) -> usize {
        let Some((_, _, Some(pl))) = self.page_parts() else { return 1 };
        let teams = ["voice", "sub"].iter().map(|k| pl.releases.iter().filter(|r| r.kind == *k).count()).max().unwrap_or(0);
        let players = pl
            .releases
            .iter()
            .map(|r| {
                let mut names: Vec<&str> = r.episodes.iter().flat_map(|e| e.sources.iter().map(|s| s.provider.as_str())).collect();
                names.sort_unstable();
                names.dedup();
                names.len()
            })
            .max()
            .unwrap_or(0);
        teams.max(players).max(2)
    }

    /// Chooses row `i` of column `col`; the page follows right away.
    fn source_pick(&mut self, col: usize, i: usize) {
        match col {
            0 => {
                if let Some(k) = self.kinds().get(i).cloned() {
                    self.set_kind(&k);
                }
            }
            1 => {
                let kind = self.page.as_ref().map(|p| p.kind.clone()).unwrap_or_default();
                if let Some(&rel) = self.teams_of(&kind).get(i) {
                    self.select_release(rel);
                }
            }
            _ => {
                if let Some(name) = self.players().get(i).cloned() {
                    if let Some(p) = &mut self.page {
                        p.provider = Some(name);
                    }
                }
            }
        }
        if let Some(p) = &mut self.page {
            p.modal = Some(Modal::Source(col.min(2)));
        }
    }

    fn related_pick(&mut self, i: usize) {
        let r = self.relations().get(i).and_then(|r| r.anime.reference());
        self.close_modal();
        if let Some(r) = r {
            self.open_related(r);
        }
    }

    fn modal_key(&mut self, k: KeyEvent) {
        let Some(m) = self.page.as_ref().and_then(|p| p.modal.clone()) else { return };
        match m {
            Modal::Related => {
                let n = self.relations().len();
                let Some(p) = &mut self.page else { return };
                match k.code {
                    KeyCode::Esc | KeyCode::Backspace => self.close_modal(),
                    KeyCode::Up | KeyCode::Char('k') => p.modal_sel = p.modal_sel.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => p.modal_sel = (p.modal_sel + 1).min(n.saturating_sub(1)),
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let i = p.modal_sel;
                        self.related_pick(i);
                    }
                    KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < n => self.related_pick(c as usize - '1' as usize),
                    _ => {}
                }
            }
            Modal::Source(col) => {
                let cols = self.source_columns();
                let (len, sel) = (cols[col].0.len(), cols[col].1);
                match k.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Backspace => self.close_modal(),
                    KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => self.open_modal(Modal::Source(col.saturating_sub(1))),
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => self.open_modal(Modal::Source((col + 1).min(2))),
                    KeyCode::Up | KeyCode::Char('k') if sel > 0 => self.source_pick(col, sel - 1),
                    KeyCode::Down | KeyCode::Char('j') if sel + 1 < len => self.source_pick(col, sel + 1),
                    KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < len => self.source_pick(col, c as usize - '1' as usize),
                    KeyCode::Char('c') => self.open_modal(Modal::Source(1)),
                    KeyCode::Char('p') => self.open_modal(Modal::Source(2)),
                    _ => {}
                }
            }
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
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.page = self.page_stack.pop(),
            KeyCode::Char('r') => {
                if !self.relations().is_empty() {
                    self.open_modal(Modal::Related);
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                p.focus = if p.focus == Focus::Episodes { Focus::Buttons } else { Focus::Episodes };
            }
            KeyCode::Char('c') => self.open_modal(Modal::Source(1)),
            KeyCode::Char('p') => self.open_modal(Modal::Source(2)),
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
            Hit::Back => self.page = self.page_stack.pop(),
            Hit::Related(r) => self.open_related(r),
            Hit::Outside => self.close_modal(),
            Hit::Inside => {}
            Hit::Item(i) => self.related_pick(i),
            Hit::Cell(col, i) => self.source_pick(col, i),
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
            vec![("←→↑↓", "серія"), ("Tab", "кнопки"), ("Ctrl+P", "пошук"), ("Ctrl+H", "нещодавні"), ("Esc", "назад")]
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

        y = self.draw_related(f, x, y, w, &mut hits);
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
            let hidden = wrapped.len().saturating_sub(max);
            more = if hidden > 0 { "розгорнути" } else if page.full_desc { "згорнути" } else { "" };
            lines.push(Line::raw(""));
            desc_rows = wrapped.len().min(max);
            let mut shown: Vec<Line> = wrapped.into_iter().take(max).collect();
            if hidden > 0 {
                // Fade the last visible line into an ellipsis and say how much is left.
                if let Some(last) = shown.last_mut() {
                    let text: String = last.spans.iter().map(|s| s.content.as_ref()).collect();
                    *last = Line::styled(format!("{}…", text.trim_end()), ui::muted());
                }
                shown.push(Line::styled(format!("▾ ще {hidden} {}", lines_word(hidden)), Style::new().fg(ACCENT)));
                desc_rows += 1;
            }
            lines.extend(shown);
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

    /// The first few related titles as a list; `r` opens all of them.
    fn draw_related(&self, f: &mut Frame, x: u16, y: u16, w: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        const SHOWN: usize = 3;
        let rels = self.relations();
        if rels.is_empty() {
            return y;
        }
        let rows = rels.len().min(SHOWN) as u16 + u16::from(rels.len() > SHOWN);
        let panel = Rect { x: x - 1, y, width: w + 2, height: rows + 4 };
        ui::fill(f, panel, SURFACE);
        let ix = x + 1;
        let iw = w.saturating_sub(2);

        let all = ui::hint("r", &format!("усі · {}", rels.len()));
        let aw = ui::width(&all);
        hits.push((Rect { x: ix + iw - aw, y: y + 1, width: aw, height: 1 }, Hit::Open(Modal::Related)));
        ui::section(f, Rect { x: ix, y: y + 1, width: iw, height: 1 }, "Сезони та пов'язані", "", all, false);

        for (i, r) in rels.iter().take(SHOWN).enumerate() {
            let row = Rect { x: ix, y: y + 3 + i as u16, width: iw, height: 1 };
            let meta = [Some(r.label().to_string()), r.anime.format.as_deref().map(str::to_uppercase), r.anime.year.map(|y| y.to_string())]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let title_w = (iw as usize).saturating_sub(meta.width() + 3);
            f.render_widget(
                Line::from(vec![
                    Span::styled(crate::style::pad(r.anime.titles.main(), title_w), ui::bold()),
                    Span::raw("   "),
                    Span::styled(meta, ui::muted()),
                ]),
                row,
            );
            if let Some(id) = r.anime.reference() {
                hits.push((row, Hit::Related(id)));
            }
        }
        if rels.len() > SHOWN {
            let row = Rect { x: ix, y: y + 3 + SHOWN as u16, width: iw, height: 1 };
            f.render_widget(Line::styled(format!("… ще {}", rels.len() - SHOWN), ui::muted()), row);
            hits.push((row, Hit::Open(Modal::Related)));
        }
        y + rows + 5
    }

    /// One panel: the chosen team and player; `c` / `p` open the picker.
    fn draw_choice(&self, f: &mut Frame, x: u16, y: u16, w: u16, hits: &mut Vec<(Rect, Hit)>) -> u16 {
        let (page, _, Some(pl)) = self.page_parts().unwrap() else { return y };
        let panel = Rect { x: x - 1, y, width: w + 2, height: 4 };
        ui::fill(f, panel, SURFACE);
        let ix = x + 1;
        let iw = w.saturating_sub(2);
        let keys = vec![ui::key("c"), Span::styled(" команда   ", ui::muted()), ui::key("p"), Span::styled(" плеєр", ui::muted())];
        ui::section(f, Rect { x: ix, y: y + 1, width: iw, height: 1 }, "Команда і плеєр", "", keys, false);

        let half = iw / 2;
        let team = match pl.releases.get(page.rel) {
            Some(r) => vec![
                Span::styled(r.team_names(), ui::bold()),
                Span::styled(
                    format!("   {} · {} еп.", r.kind_label(), r.last_episode.map(fmt_num).unwrap_or_else(|| "?".into())),
                    ui::muted(),
                ),
            ],
            None => vec![Span::styled("—", ui::muted())],
        };
        let player = match self.current_source() {
            Some(s) => vec![
                Span::styled(s.provider.clone(), ui::bold()),
                Span::styled(if play::supports_mpv(&s.provider) { "   браузер і mpv" } else { "   лише браузер" }, ui::muted()),
            ],
            None => vec![Span::styled("—", ui::muted())],
        };
        let left = Rect { x: ix, y: y + 2, width: half, height: 1 };
        let right = Rect { x: ix + half, y: y + 2, width: iw - half, height: 1 };
        f.render_widget(Line::from(team), left);
        f.render_widget(Line::from(player), right);
        hits.push((panel, Hit::Open(Modal::Source(1))));
        hits.push((Rect { height: 2, ..right }, Hit::Open(Modal::Source(2))));
        y + 5
    }

    /// Rows the grid needs and its column count at width `w`.
    /// Rows for the episode grid and its column count. The rows fit the
    /// longest release of the title, so switching teams keeps the layout still.
    fn episode_rows(&self, w: u16) -> (usize, usize) {
        let Some((p, _, Some(pl))) = self.page_parts() else { return (1, 1) };
        let cols = pl.releases.get(p.rel).map_or(1, |rel| grid_shape(rel, w.saturating_sub(2)).1);
        let rows = pl.releases.iter().map(|r| grid_shape(r, w.saturating_sub(2)).0).max().unwrap_or(1);
        (rows, cols)
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
        let Some(m) = page.modal.clone() else { return vec![] };
        f.buffer_mut().set_style(area, Style::new().fg(FAINT));
        match m {
            Modal::Related => {
                let items = self
                    .relations()
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| {
                        let meta = [Some(r.label().to_string()), r.anime.format.as_deref().map(str::to_uppercase), r.anime.year.map(|y| y.to_string())]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ");
                        let mut it = ui::Item::new(r.anime.titles.main(), meta);
                        it.key = (i < 9).then(|| (i + 1).to_string());
                        it
                    })
                    .collect();
                let overlay = ui::Overlay {
                    title: "Сезони та пов'язані".into(),
                    aside: String::new(),
                    filter: None,
                    items,
                    sel: page.modal_sel,
                    empty: "нічого немає".into(),
                    footer: vec![("↑↓", "вибір"), ("⏎", "відкрити"), ("Esc", "закрити")],
                    width: 72,
                };
                let layout = overlay.draw(f, area);
                let mut hits = vec![(area, Hit::Outside), (layout.rect, Hit::Inside)];
                hits.extend(layout.rows.into_iter().map(|(r, i)| (r, Hit::Item(i))));
                hits
            }
            Modal::Source(active) => self.draw_source_modal(f, area, active),
        }
    }

    /// Track | team | player side by side. Moving in a column applies at once.
    fn draw_source_modal(&self, f: &mut Frame, area: Rect, active: usize) -> Vec<(Rect, Hit)> {
        let cols = self.source_columns();
        let w = 100.min(area.width.saturating_sub(4));
        // Sized for the longest column any choice can produce, so moving
        // through tracks and teams never resizes the modal.
        let rows_h = self.source_rows_max().max(cols.iter().map(|(items, _)| items.len()).max().unwrap_or(1)).clamp(1, area.height.saturating_sub(12) as usize) as u16;
        let h = rows_h + 8;
        let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + area.height.saturating_sub(h) / 3, width: w, height: h };
        f.render_widget(Clear, rect);
        ui::fill(f, rect, ui::OVERLAY);
        let ix = rect.x + 2;
        let iw = rect.width.saturating_sub(4);
        let ep = self.current().map(|(_, e)| format!("{} еп.", e.label())).unwrap_or_default();
        ui::section(f, Rect { x: ix, y: rect.y + 1, width: iw, height: 1 }, "Команда і плеєр", "", vec![Span::styled(ep, ui::muted())], true);

        let gap = 3;
        let track_w = 22;
        let player_w = 22;
        let team_w = iw.saturating_sub(track_w + player_w + 2 * gap);
        let xs = [(ix, track_w), (ix + track_w + gap, team_w), (ix + track_w + gap + team_w + gap, player_w)];
        let titles = ["Трек", "Команда", "Плеєр"];
        let mut hits = vec![(area, Hit::Outside), (rect, Hit::Inside)];
        for (c, ((cx, cw), (items, sel))) in xs.iter().zip(cols.iter()).enumerate() {
            let on = c == active;
            ui::section(f, Rect { x: *cx, y: rect.y + 3, width: *cw, height: 1 }, titles[c], "", vec![], on);
            if on {
                ui::focus_bar(f, cx - 2, rect.y + 3, rows_h + 2);
            }
            let list = Rect { x: cx - 1, y: rect.y + 5, width: cw + 1, height: rows_h };
            for (r, i) in ui::rows(f, list, items, *sel, ui::OVERLAY, on) {
                hits.push((r, Hit::Cell(c, i)));
            }
        }
        let footer = ui::hints(&[("←→", "стовпчик"), ("↑↓", "вибір"), ("⏎", "готово")]);
        f.render_widget(Line::from(footer), Rect { x: ix, y: rect.y + h - 2, width: iw, height: 1 });
        hits
    }
}

fn lines_word(n: usize) -> &'static str {
    match (n % 10, n % 100) {
        (1, r) if r != 11 => "рядок",
        (2..=4, r) if !(12..=14).contains(&r) => "рядки",
        _ => "рядків",
    }
}

fn cell_width(rel: &Release) -> usize {
    rel.episodes.iter().map(|e| e.label().width()).max().unwrap_or(1) + 5
}

fn grid_shape(rel: &Release, w: u16) -> (usize, usize) {
    let cols = (w as usize / cell_width(rel)).max(1);
    (rel.episodes.len().div_ceil(cols).max(1), cols)
}
