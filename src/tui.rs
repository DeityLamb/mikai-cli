//! Agent-style TUI: a scrollable transcript of commands and their output, a
//! command prompt at the bottom and a Ctrl+P palette.
//!
//! The prompt accepts only `/commands`. `/show` opens an anime as the current
//! session; episode and list commands then act on it. The transcript, the
//! prompt history and the open session are saved between runs.

use std::{
    fs,
    io::stdout,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, Local, Utc};
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind},
        execute,
    },
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    api::Client,
    cli,
    config::Config,
    models::*,
    view::{self, Lines, SLine, Seg, Tone, blank, dim, err, plain},
};

mod page;

mod ui;

use ui::{ACCENT, FAINT, GREEN, MUTED, RED, SURFACE, SURFACE_ALT, YELLOW};
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MAX_ENTRIES: usize = 300;
const MAX_INPUTS: usize = 500;

// ---------- commands ----------

struct Spec {
    name: &'static str,
    aliases: &'static [&'static str],
    args: &'static str,
    help: &'static str,
}

const COMMANDS: &[Spec] = &[
    Spec { name: "search", aliases: &["s"], args: "<назва>", help: "пошук аніме" },
    Spec { name: "show", aliases: &["a", "open"], args: "<аніме|#N>", help: "відкрити аніме як сесію" },
    Spec { name: "eps", aliases: &["e"], args: "[команда|#N]", help: "релізи, а з командою — серії" },
    Spec { name: "watch", aliases: &["w"], args: "<серія> [команда] [плеєр] [--mpv]", help: "відкрити серію в браузері" },
    Spec { name: "play", aliases: &["p", "mpv"], args: "<серія> [команда] [плеєр]", help: "відкрити серію в mpv" },
    Spec { name: "next", aliases: &["n"], args: "[--mpv]", help: "наступна серія" },
    Spec { name: "view", aliases: &["v"], args: "[аніме|#N]", help: "сторінка перегляду тайтлу" },
    Spec { name: "resume", aliases: &["continue", "c"], args: "[аніме]", help: "продовжити: сторінка на наступній серії" },
    Spec { name: "ep", aliases: &[], args: "<N>", help: "переглянуто до серії N" },
    Spec { name: "status", aliases: &["st"], args: "<статус>", help: "дивлюсь, заплановано, переглянуто…" },
    Spec { name: "score", aliases: &[], args: "<0.5–10|0>", help: "оцінка (0 — прибрати)" },
    Spec { name: "rm", aliases: &[], args: "", help: "прибрати зі списків" },
    Spec { name: "site", aliases: &[], args: "", help: "сторінка аніме на mikai.me" },
    Spec { name: "close", aliases: &[], args: "", help: "закрити поточне аніме" },
    Spec { name: "random", aliases: &[], args: "", help: "випадкове аніме" },
    Spec { name: "schedule", aliases: &["sch"], args: "[день]", help: "розклад виходу серій" },
    Spec { name: "updates", aliases: &["new"], args: "[кількість]", help: "щойно додані серії" },
    Spec { name: "list", aliases: &["ls"], args: "[статус]", help: "мої списки" },
    Spec { name: "history", aliases: &[], args: "", help: "історія переглядів" },
    Spec { name: "notif", aliases: &[], args: "[all|read]", help: "сповіщення" },
    Spec { name: "translit", aliases: &[], args: "<текст>", help: "українська транслітерація" },
    Spec { name: "login", aliases: &[], args: "[mk_…]", help: "увійти (за посиланням або ключем)" },
    Spec { name: "logout", aliases: &[], args: "", help: "вийти" },
    Spec { name: "whoami", aliases: &["me"], args: "", help: "акаунт і ліміти" },
    Spec { name: "clear", aliases: &[], args: "", help: "очистити історію" },
    Spec { name: "help", aliases: &["?"], args: "", help: "довідка" },
    Spec { name: "quit", aliases: &["q", "exit"], args: "", help: "вихід" },
];

fn find_command(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|c| c.name == name || c.aliases.contains(&name))
}

fn help_lines() -> Lines {
    let mut out = vec![SLine::of("Команди", Tone::Bold)];
    for c in COMMANDS {
        let names = std::iter::once(c.name).chain(c.aliases.iter().copied()).map(|n| format!("/{n}")).collect::<Vec<_>>().join(" ");
        out.push(SLine::new().col(&names, 18, Tone::Accent).col(c.args, 26, Tone::Plain).push(c.help, Tone::Dim).nowrap());
    }
    out.push(blank());
    out.push(dim("#N посилається на рядок останнього списку: /show #2, /eps #1. Аніме — id, mal:21, посилання або назва."));
    out.push(blank());
    out.push(SLine::of("Клавіші", Tone::Bold));
    for (k, d) in [
        ("Ctrl+P", "меню: дії, пошук аніме, вибір серії"),
        ("Ctrl+H", "нещодавно переглянуті — продовжити з наступної серії"),
        ("Tab", "доповнити команду"),
        ("↑ ↓", "попередні команди"),
        ("PgUp PgDn, колесо", "гортати історію (Esc — донизу)"),
        ("Ctrl+↑ Ctrl+↓", "вибрати блок; Enter — згорнути/розгорнути"),
        ("Ctrl+O", "згорнути всі минулі блоки / розгорнути"),
        ("клік по заголовку", "згорнути/розгорнути блок"),
        ("Ctrl+C", "очистити ввід / вихід"),
    ] {
        out.push(SLine::new().col(k, 18, Tone::Accent).push(d, Tone::Dim));
    }
    out
}

fn welcome_lines() -> Lines {
    vec![
        SLine::of("mikai", Tone::Accent).push("  аніме з українським озвученням у терміналі", Tone::Dim),
        blank(),
        plain("Почніть з ").push("/search назва", Tone::Accent).push(" або натисніть ", Tone::Plain).push("Ctrl+P", Tone::Accent).push(".", Tone::Plain),
        plain("Сторінка тайтлу з вибором серії та плеєра — ").push("/view назва", Tone::Accent).push(".", Tone::Plain),
        plain("Відкрите через ").push("/show", Tone::Accent).push(" аніме стає сесією: ", Tone::Plain).push("/eps", Tone::Accent).push(", ", Tone::Plain).push("/watch 3", Tone::Accent).push(", ", Tone::Plain).push("/next", Tone::Accent).push(" діють на нього.", Tone::Plain),
        plain("Усі команди — ").push("/help", Tone::Accent).push(". Історія зберігається між запусками.", Tone::Plain),
    ]
}

// ---------- state ----------

/// The anime the prompt is working on.
#[derive(Clone)]
struct Session {
    r: String,
    title: String,
    url: Option<String>,
    episodes: EpisodesInfo,
    /// Last release used, as a release id.
    team: Option<String>,
    last_ep: Option<f64>,
    player: Option<Player>,
    /// `None` — unknown (not logged in), `Some(None)` — in no list.
    entry: Option<Option<ListEntry>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SavedSession {
    r: String,
    title: String,
    team: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    /// Empty for notes and separators that are not commands.
    cmd: String,
    at: DateTime<Utc>,
    lines: Lines,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    collapsed: bool,
    #[serde(skip)]
    pending: bool,
    #[serde(skip)]
    id: u64,
    #[serde(skip)]
    cache: Option<(u16, Vec<Line<'static>>)>,
}

/// A title that was watched, for Ctrl+H and `/resume`.
#[derive(Serialize, Deserialize, Clone)]
struct Recent {
    r: String,
    title: String,
    ep: f64,
    /// Release id.
    team: Option<String>,
    team_name: Option<String>,
    at: DateTime<Utc>,
}

const MAX_RECENT: usize = 100;

fn ago(at: DateTime<Utc>) -> String {
    let mins = (Utc::now() - at).num_minutes();
    match mins {
        ..=0 => "щойно".into(),
        1..=59 => format!("{mins} хв тому"),
        60..=1439 => format!("{} год тому", mins / 60),
        1440..=2879 => "вчора".into(),
        2880..=43199 => format!("{} дн. тому", mins / 1440),
        _ => at.with_timezone(&Local).format("%d.%m.%Y").to_string(),
    }
}

#[derive(Serialize, Deserialize, Default)]
struct SavedState {
    entries: Vec<Entry>,
    inputs: Vec<String>,
    session: Option<SavedSession>,
    /// Targets of `#N` in the last anime list and release list.
    #[serde(default)]
    anime_refs: Vec<String>,
    #[serde(default)]
    release_refs: Vec<String>,
    #[serde(default)]
    recent: Vec<Recent>,
}

fn state_path() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("mikai").join("session.json"))
}

/// What a finished command changes besides printing.
#[derive(Default)]
struct Outcome {
    lines: Lines,
    anime_refs: Option<Vec<AnimeShort>>,
    release_refs: Option<Vec<String>>,
    open: Option<Session>,
    close: bool,
    player: Option<(String, Player)>,
    entry: Option<(String, Option<ListEntry>)>,
    team: Option<(String, String)>,
    last_ep: Option<(String, f64)>,
    /// `Some(None)` — logged out.
    auth: Option<Option<(String, String)>>,
    /// Open the watch page for this anime.
    page: Option<AnimeDetail>,
}

enum Msg {
    Append(u64, Lines),
    Done(u64, Result<Outcome>),
    Found(u64, Result<Vec<AnimeShort>>),
    Player(String, Result<Player>),
    History(Result<Vec<serde_json::Value>>),
    Teams(Result<Vec<Team>>),
}

#[derive(Clone)]
enum PAction {
    Run(String),
    Recent,
    Search,
    Releases,
    Episodes(usize),
}

struct PItem {
    label: String,
    hint: String,
    action: PAction,
}

#[derive(PartialEq)]
enum PMode {
    Actions,
    Recent,
    Search,
    Releases,
    Episodes(usize),
}

struct Palette {
    mode: PMode,
    query: String,
    sel: usize,
    results: Vec<AnimeShort>,
    searched: String,
    typed_at: Instant,
    generation: u64,
}

struct App {
    client: Client,
    user: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    entries: Vec<Entry>,
    next_id: u64,
    scroll: usize,
    total_lines: usize,
    /// Block picked with Ctrl+↑/↓, by index in `entries`.
    focus: Option<usize>,
    reveal_focus: bool,
    /// Entry index of every visible transcript row, for mouse clicks.
    row_owner: Vec<Option<usize>>,
    transcript_y: u16,

    input: String,
    cursor: usize,
    inputs: Vec<String>,
    recall: Option<usize>,
    draft: String,
    ac_sel: usize,

    session: Option<Session>,
    anime_refs: Vec<String>,
    release_refs: Vec<String>,
    recent: Vec<Recent>,
    /// Team id → name, for releases known only by id (`t_35:voice`).
    team_names: std::collections::HashMap<i64, String>,

    palette: Option<Palette>,
    page: Option<page::Page>,
    tick: usize,
    loading: usize,
    quit: bool,
}

pub fn run(client: Client, user: Option<String>) -> Result<()> {
    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    let mut app = App::new(client, user);
    let result = app.run(&mut terminal);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    app.save();
    result
}

impl App {
    fn new(client: Client, user: Option<String>) -> Self {
        let (tx, rx) = mpsc::channel();
        let saved: SavedState = state_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut app = Self {
            client,
            user,
            tx,
            rx,
            entries: vec![],
            next_id: 1,
            scroll: 0,
            total_lines: 0,
            focus: None,
            reveal_focus: false,
            row_owner: vec![],
            transcript_y: 0,
            input: String::new(),
            cursor: 0,
            inputs: saved.inputs,
            recall: None,
            draft: String::new(),
            ac_sel: 0,
            session: None,
            anime_refs: saved.anime_refs,
            release_refs: saved.release_refs,
            recent: saved.recent,
            team_names: Default::default(),
            palette: None,
            page: None,
            tick: 0,
            loading: 0,
            quit: false,
        };
        let first_run = saved.entries.is_empty();
        for mut e in saved.entries {
            e.id = app.bump();
            app.entries.push(e);
        }
        if first_run {
            app.note(welcome_lines());
        } else {
            app.note(vec![dim(format!("── {} ──", Local::now().format("%d.%m.%Y %H:%M")))]);
        }
        if let Some(s) = saved.session {
            app.session = Some(Session {
                r: s.r.clone(),
                title: s.title,
                url: None,
                episodes: EpisodesInfo::default(),
                team: s.team,
                last_ep: None,
                player: None,
                entry: None,
            });
            // Refresh the restored session quietly.
            let r = s.r.clone();
            app.spawn_raw(move |c| Msg::Player(r.clone(), c.player(&r)));
        }
        app
    }

    fn bump(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// Recent titles saved by other running instances, merged into ours.
    fn sync_recent(&mut self) {
        let Some(path) = state_path() else { return };
        let Some(disk) = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str::<SavedState>(&s).ok()) else { return };
        for rec in disk.recent {
            self.remember(rec);
        }
    }

    fn save(&mut self) {
        // Several mikai windows share one state file: never drop what another
        // one has watched. The rest of the state is per window, last write wins.
        self.sync_recent();
        let Some(path) = state_path() else { return };
        let skip = self.entries.len().saturating_sub(MAX_ENTRIES);
        let state = json!({
            "entries": self.entries[skip..].iter().filter(|e| !e.pending).collect::<Vec<_>>(),
            "inputs": &self.inputs[self.inputs.len().saturating_sub(MAX_INPUTS)..],
            "anime_refs": &self.anime_refs,
            "release_refs": &self.release_refs,
            "recent": &self.recent,
            "session": self.session.as_ref().map(|s| SavedSession { r: s.r.clone(), title: s.title.clone(), team: s.team.clone() }),
        });
        let _ = fs::create_dir_all(path.parent().unwrap());
        // Write to a side file and rename, so a reader never sees half a file.
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        if fs::write(&tmp, state.to_string()).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }

    fn note(&mut self, lines: Lines) {
        let id = self.bump();
        self.entries.push(Entry { cmd: String::new(), at: Utc::now(), lines, pending: false, collapsed: false, id, cache: None });
        self.scroll = 0;
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(80))? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => self.on_key(k),
                    Event::Mouse(m) => match m.kind {
                        MouseEventKind::ScrollUp => self.scroll_by(3),
                        MouseEventKind::ScrollDown => self.scroll_by(-3),
                        MouseEventKind::Down(event::MouseButton::Left) if self.palette.is_none() && self.page.is_some() => {
                            self.page_click(m.column, m.row);
                        }
                        MouseEventKind::Down(event::MouseButton::Left) if self.palette.is_none() => {
                            let row = m.row.checked_sub(self.transcript_y).map(usize::from);
                            if let Some(i) = row.and_then(|r| self.row_owner.get(r).copied().flatten()) {
                                self.toggle(i);
                            }
                        }
                        _ => {}
                    },
                    Event::Paste(s) => self.insert(&s.replace(['\n', '\r'], " ")),
                    _ => {}
                }
            }
            while let Ok(msg) = self.rx.try_recv() {
                self.on_msg(msg);
            }
            self.tick += 1;
            self.palette_search_tick();
        }
        Ok(())
    }

    // ---------- running commands ----------

    fn spawn_raw(&mut self, f: impl FnOnce(&Client) -> Msg + Send + 'static) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.loading += 1;
        thread::spawn(move || {
            let _ = tx.send(f(&client));
        });
    }

    fn submit(&mut self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.inputs.last() != Some(&text) {
            self.inputs.push(text.clone());
        }
        self.recall = None;
        self.scroll = 0;

        let body = text.strip_prefix('/').unwrap_or(&text);
        let (name, args) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
        let args = args.trim().to_string();
        let display = format!("/{}", body.trim());

        let Some(spec) = find_command(&name.to_lowercase()) else {
            let id = self.bump();
            let lines = vec![err(format!("Невідома команда «{name}».")).push("  /help — список, Ctrl+P — меню", Tone::Dim)];
            self.entries.push(Entry { cmd: display, at: Utc::now(), lines, pending: false, collapsed: false, id, cache: None });
            return;
        };

        // Commands that only touch local state.
        let local: Option<Lines> = match spec.name {
            "quit" => {
                self.quit = true;
                return;
            }
            "clear" => {
                self.entries.clear();
                self.focus = None;
                self.save();
                return;
            }
            "help" => Some(help_lines()),
            "close" => Some(match self.session.take() {
                Some(s) => vec![dim(format!("Закрито «{}»", s.title))],
                None => vec![dim("Немає відкритого аніме")],
            }),
            _ => None,
        };
        let id = self.bump();
        if let Some(lines) = local {
            self.entries.push(Entry { cmd: display, at: Utc::now(), lines, pending: false, collapsed: false, id, cache: None });
            return;
        }

        self.entries.push(Entry { cmd: display, at: Utc::now(), lines: vec![], pending: true, collapsed: false, id, cache: None });
        let snap = Snap {
            client: self.client.clone(),
            session: self.session.clone(),
            anime_refs: self.anime_refs.clone(),
            release_refs: self.release_refs.clone(),
            recent: self.recent.clone(),
            tx: self.tx.clone(),
            id,
        };
        let name = spec.name;
        let tx = self.tx.clone();
        self.loading += 1;
        thread::spawn(move || {
            let res = exec(name, &args, &snap);
            let _ = tx.send(Msg::Done(id, res));
        });
    }

    fn entry_mut(&mut self, id: u64) -> Option<&mut Entry> {
        self.entries.iter_mut().rev().find(|e| e.id == id)
    }

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Append(id, lines) => {
                if let Some(e) = self.entry_mut(id) {
                    e.lines.extend(lines);
                    e.cache = None;
                }
            }
            Msg::Done(id, res) => {
                self.loading = self.loading.saturating_sub(1);
                let lines = match res {
                    Ok(o) => self.apply(o),
                    Err(e) => vec![err(format!("{e:#}"))],
                };
                if let Some(e) = self.entry_mut(id) {
                    e.lines.extend(lines);
                    e.pending = false;
                    e.cache = None;
                }
                self.save();
            }
            Msg::Found(generation, res) => {
                self.loading = self.loading.saturating_sub(1);
                if let Some(p) = &mut self.palette {
                    if p.generation == generation {
                        p.results = res.unwrap_or_default();
                        p.sel = 0;
                    }
                }
            }
            Msg::History(res) => {
                self.loading = self.loading.saturating_sub(1);
                if let Ok(items) = res {
                    for it in items {
                        let anime = view::anime_of(&it);
                        let Some(r) = anime.reference() else { continue };
                        let at = ["watchedAt", "updatedAt", "createdAt"]
                            .iter()
                            .find_map(|k| it.get(*k).and_then(|v| v.as_str()))
                            .and_then(view::parse_time)
                            .unwrap_or_else(Utc::now);
                        let rec = Recent {
                            r,
                            title: anime.titles.main().to_string(),
                            ep: it.get("episode").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            team: it.get("releaseId").and_then(|v| v.as_str()).map(str::to_string),
                            team_name: None,
                            at,
                        };
                        self.remember(rec);
                    }
                }
            }
            Msg::Teams(res) => {
                self.loading = self.loading.saturating_sub(1);
                for t in res.unwrap_or_default() {
                    if let Some(id) = t.id {
                        self.team_names.insert(id, t.name);
                    }
                }
            }
            Msg::Player(r, res) => {
                self.loading = self.loading.saturating_sub(1);
                if let (Ok(p), Some(s)) = (res, &mut self.session) {
                    if s.r == r {
                        s.title = p.anime.titles.main().to_string();
                        s.url = p.anime.url();
                        s.player = Some(p);
                    }
                }
            }
        }
    }

    fn apply(&mut self, o: Outcome) -> Lines {
        if let Some(v) = o.anime_refs {
            self.anime_refs = v.iter().filter_map(AnimeShort::reference).collect();
        }
        if let Some(v) = o.release_refs {
            self.release_refs = v;
        }
        if o.close {
            self.session = None;
        }
        if let Some(s) = o.open {
            self.session = Some(s);
        }
        if let Some(s) = &mut self.session {
            if let Some((r, p)) = o.player {
                if s.r == r {
                    s.player = Some(p);
                }
            }
            if let Some((r, e)) = o.entry {
                if s.r == r {
                    s.entry = Some(e);
                }
            }
            if let Some((r, t)) = o.team {
                if s.r == r {
                    s.team = Some(t);
                }
            }
            if let Some((r, n)) = o.last_ep {
                if s.r == r {
                    s.last_ep = Some(n);
                    let team_name = s.team.as_ref().and_then(|t| {
                        s.player.as_ref().and_then(|p| p.releases.iter().find(|x| &x.id == t)).map(|x| x.team_names())
                    });
                    let rec = Recent { r, title: s.title.clone(), ep: n, team: s.team.clone(), team_name, at: Utc::now() };
                    self.remember(rec);
                }
            }
        }
        if let Some(d) = o.page {
            self.open_page(d);
        }
        match o.auth {
            Some(Some((key, user))) => {
                self.client.set_key(Some(key));
                self.user = Some(user);
            }
            Some(None) => {
                self.client.set_key(None);
                self.user = None;
                if let Some(s) = &mut self.session {
                    s.entry = None;
                }
            }
            None => {}
        }
        o.lines
    }

    // ---------- input ----------

    fn insert(&mut self, s: &str) {
        let byte = self.byte_at(self.cursor);
        self.input.insert_str(byte, s);
        self.cursor += s.chars().count();
        self.ac_sel = 0;
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.input.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(self.input.len())
    }

    fn set_input(&mut self, s: String) {
        self.cursor = s.chars().count();
        self.input = s;
        self.ac_sel = 0;
    }

    fn completions(&self) -> Vec<&'static Spec> {
        let Some(typed) = self.input.strip_prefix('/') else { return vec![] };
        if typed.contains(' ') {
            return vec![];
        }
        let typed = typed.to_lowercase();
        COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(&typed) || c.aliases.iter().any(|a| a.starts_with(&typed)))
            .collect()
    }

    fn scroll_by(&mut self, delta: i64) {
        self.scroll = (self.scroll as i64 + delta).clamp(0, self.total_lines as i64) as usize;
    }

    fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('p') {
            self.toggle_palette();
            return;
        }
        if ctrl && k.code == KeyCode::Char('h') {
            if matches!(self.palette.as_ref().map(|p| &p.mode), Some(PMode::Recent)) {
                self.palette = None;
            } else {
                self.open_recent();
            }
            return;
        }
        if self.palette.is_some() {
            self.palette_key(k);
            return;
        }
        if self.page.is_some() {
            self.page_key(k);
            return;
        }
        if self.block_key(k) {
            return;
        }
        let comps = self.completions();
        match k.code {
            KeyCode::Char('c') if ctrl => {
                if self.input.is_empty() {
                    self.quit = true;
                } else {
                    self.set_input(String::new());
                }
            }
            KeyCode::Char('d') if ctrl => self.quit = true,
            KeyCode::Char('l') if ctrl => self.submit("/clear".into()),
            KeyCode::Char('u') if ctrl => {
                let byte = self.byte_at(self.cursor);
                self.input.replace_range(..byte, "");
                self.cursor = 0;
            }
            KeyCode::Char('w') if ctrl => {
                let before: String = self.input.chars().take(self.cursor).collect();
                let trimmed = before.trim_end();
                let cut = trimmed.rfind(' ').map(|i| i + 1).unwrap_or(0);
                let rest: String = self.input.chars().skip(self.cursor).collect();
                let new_before = before[..cut].to_string();
                self.cursor = new_before.chars().count();
                self.input = new_before + &rest;
            }
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.input.chars().count(),
            KeyCode::Char(c) => self.insert(&c.to_string()),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let byte = self.byte_at(self.cursor);
                    self.input.remove(byte);
                    self.ac_sel = 0;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.chars().count() {
                    let byte = self.byte_at(self.cursor);
                    self.input.remove(byte);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.input.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.chars().count(),
            KeyCode::Tab if !comps.is_empty() => {
                let c = comps[self.ac_sel.min(comps.len() - 1)];
                self.set_input(format!("/{} ", c.name));
            }
            KeyCode::Enter => {
                // Enter on a half-typed command picks the highlighted completion.
                if !comps.is_empty() && !comps.iter().any(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()).any(|n| self.input[1..].eq_ignore_ascii_case(n))) {
                    let c = comps[self.ac_sel.min(comps.len() - 1)];
                    if c.args.starts_with('<') {
                        self.set_input(format!("/{} ", c.name));
                        return;
                    }
                    self.set_input(String::new());
                    self.submit(format!("/{}", c.name));
                    return;
                }
                let text = std::mem::take(&mut self.input);
                self.cursor = 0;
                self.submit(text);
            }
            KeyCode::Up if k.modifiers.contains(KeyModifiers::SHIFT) => self.scroll_by(1),
            KeyCode::Down if k.modifiers.contains(KeyModifiers::SHIFT) => self.scroll_by(-1),
            KeyCode::Up if !comps.is_empty() => self.ac_sel = self.ac_sel.saturating_sub(1),
            KeyCode::Down if !comps.is_empty() => self.ac_sel = (self.ac_sel + 1).min(comps.len() - 1),
            KeyCode::Up => {
                if self.inputs.is_empty() {
                    return;
                }
                let i = match self.recall {
                    None => {
                        self.draft = self.input.clone();
                        self.inputs.len() - 1
                    }
                    Some(i) => i.saturating_sub(1),
                };
                self.recall = Some(i);
                self.set_input(self.inputs[i].clone());
            }
            KeyCode::Down => match self.recall {
                Some(i) if i + 1 < self.inputs.len() => {
                    self.recall = Some(i + 1);
                    self.set_input(self.inputs[i + 1].clone());
                }
                Some(_) => {
                    self.recall = None;
                    let draft = std::mem::take(&mut self.draft);
                    self.set_input(draft);
                }
                None => {}
            },
            KeyCode::PageUp => self.scroll_by(15),
            KeyCode::PageDown => self.scroll_by(-15),
            KeyCode::Esc => {
                if self.scroll > 0 {
                    self.scroll = 0;
                } else {
                    self.set_input(String::new());
                }
            }
            _ => {}
        }
    }

    // ---------- blocks ----------

    fn is_block(&self, i: usize) -> bool {
        self.entries.get(i).is_some_and(|e| !e.cmd.is_empty()) && !folded_into_next(&self.entries, i)
    }

    fn toggle(&mut self, i: usize) {
        if let Some(e) = self.entries.get_mut(i) {
            if !e.cmd.is_empty() && !e.pending {
                e.collapsed = !e.collapsed;
                self.save();
            }
        }
    }

    fn move_focus(&mut self, up: bool) {
        let n = self.entries.len();
        let mut i = match (self.focus, up) {
            (None, true) => n,
            (None, false) => return,
            (Some(i), _) => i,
        };
        loop {
            if up {
                if i == 0 {
                    return;
                }
                i -= 1;
            } else {
                i += 1;
                if i >= n {
                    self.focus = None;
                    self.scroll = 0;
                    return;
                }
            }
            if self.is_block(i) {
                self.focus = Some(i);
                self.reveal_focus = true;
                return;
            }
        }
    }

    /// Keys for picking and folding blocks. Returns `true` when handled.
    fn block_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Up if ctrl => self.move_focus(true),
            KeyCode::Down if ctrl => self.move_focus(false),
            KeyCode::Char('o') if ctrl => {
                // Fold every block but the last; if all are folded already, unfold all.
                let last = self.entries.iter().rposition(|e| !e.cmd.is_empty());
                let fold = self.entries.iter().enumerate().any(|(i, e)| !e.cmd.is_empty() && !e.collapsed && Some(i) != last);
                for (i, e) in self.entries.iter_mut().enumerate() {
                    if !e.cmd.is_empty() && !e.pending {
                        e.collapsed = fold && Some(i) != last;
                    }
                }
                self.save();
            }
            _ => {
                let Some(i) = self.focus else { return false };
                match k.code {
                    KeyCode::Up => self.move_focus(true),
                    KeyCode::Down => self.move_focus(false),
                    KeyCode::Enter | KeyCode::Char(' ') => self.toggle(i),
                    KeyCode::Left => {
                        if !self.entries[i].collapsed {
                            self.toggle(i);
                        }
                    }
                    KeyCode::Right => {
                        if self.entries[i].collapsed {
                            self.toggle(i);
                        }
                    }
                    KeyCode::Esc => {
                        self.focus = None;
                        self.scroll = 0;
                    }
                    // Anything else goes back to typing.
                    _ => {
                        self.focus = None;
                        return false;
                    }
                }
                self.reveal_focus = true;
            }
        }
        true
    }

    /// Adds or refreshes a recent title; newer records win.
    fn remember(&mut self, rec: Recent) {
        if let Some(old) = self.recent.iter().find(|x| x.r == rec.r) {
            if old.at > rec.at {
                return;
            }
        }
        let team_name = rec.team_name.clone().or_else(|| {
            self.recent.iter().find(|x| x.r == rec.r && x.team == rec.team).and_then(|x| x.team_name.clone())
        });
        self.recent.retain(|x| x.r != rec.r);
        self.recent.push(Recent { team_name, ..rec });
        self.recent.sort_by(|a, b| b.at.cmp(&a.at));
        self.recent.truncate(MAX_RECENT);
    }

    fn open_recent(&mut self) {
        self.sync_recent();
        self.palette = Some(Palette {
            mode: PMode::Recent,
            query: String::new(),
            sel: 0,
            results: vec![],
            searched: String::new(),
            typed_at: Instant::now(),
            generation: 0,
        });
        if self.client.has_key() {
            self.spawn_raw(|c| Msg::History(c.history(1, 50).map(|p| p.items)));
        }
        if self.team_names.is_empty() {
            self.spawn_raw(|c| {
                let mut all = vec![];
                for page in 1..=10 {
                    match c.teams(page) {
                        Ok(p) => {
                            let last = p.pages.is_none_or(|n| u64::from(page) >= n);
                            all.extend(p.items);
                            if last {
                                break;
                            }
                        }
                        Err(e) => return Msg::Teams(Err(e)),
                    }
                }
                Msg::Teams(Ok(all))
            });
        }
    }

    /// `Inari`, or for a bare release id like `t_35:voice` the team's name.
    fn recent_team(&self, x: &Recent) -> Option<String> {
        if let Some(n) = &x.team_name {
            return Some(n.clone());
        }
        let id = x.team.as_deref()?;
        let (team, kind) = id.split_once(':').unwrap_or((id, ""));
        let name = team.strip_prefix("t_").and_then(|n| n.parse::<i64>().ok()).and_then(|n| self.team_names.get(&n).cloned());
        Some(match name {
            Some(n) if kind == "sub" => format!("{n} (суб.)"),
            Some(n) => n,
            None => kind_label(kind).to_string(),
        })
    }

    // ---------- palette ----------

    fn toggle_palette(&mut self) {
        if self.palette.take().is_none() {
            self.palette = Some(Palette {
                mode: PMode::Actions,
                query: String::new(),
                sel: 0,
                results: vec![],
                searched: String::new(),
                typed_at: Instant::now(),
                generation: 0,
            });
        }
    }

    fn palette_items(&self) -> Vec<PItem> {
        let Some(p) = &self.palette else { return vec![] };
        let run = |label: &str, hint: &str, cmd: &str| PItem { label: label.into(), hint: hint.into(), action: PAction::Run(cmd.into()) };
        let items: Vec<PItem> = match &p.mode {
            PMode::Recent => self
                .recent
                .iter()
                .map(|x| {
                    let team = self.recent_team(x).unwrap_or_default();
                    let hint = [Some(format!("{} еп.", fmt_num(x.ep))), (!team.is_empty()).then_some(team), Some(ago(x.at))]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    PItem { label: x.title.clone(), hint, action: PAction::Run(format!("/resume {}", x.r)) }
                })
                .collect(),
            PMode::Actions => {
                let mut v = vec![
                    PItem { label: "Пошук аніме…".into(), hint: "/search".into(), action: PAction::Search },
                    PItem { label: "Продовжити перегляд…".into(), hint: "Ctrl+H".into(), action: PAction::Recent },
                ];
                if let Some(s) = &self.session {
                    v.push(PItem { label: format!("Серії «{}»…", s.title), hint: "/watch".into(), action: PAction::Releases });
                    v.push(run("Сторінка перегляду", "/view", "/view"));
                    v.push(run("Дивитись наступну серію", "/next", "/next"));
                    if crate::play::mpv_available() {
                        v.push(run("Наступна серія в mpv", "/next --mpv", "/next --mpv"));
                    }
                    v.push(run("Картка аніме", "/show", &format!("/show {}", s.r)));
                    v.push(run("Сторінка на mikai.me", "/site", "/site"));
                    if self.client.has_key() {
                        for st in LIST_STATUSES {
                            v.push(run(&format!("У список: {}", list_status_label(st)), "/status", &format!("/status {st}")));
                        }
                    }
                    v.push(run("Закрити аніме", "/close", "/close"));
                }
                v.extend([
                    run("Розклад на сьогодні", "/schedule today", "/schedule today"),
                    run("Розклад на тиждень", "/schedule", "/schedule"),
                    run("Новинки", "/updates", "/updates"),
                    run("Випадкове аніме", "/random", "/random"),
                ]);
                if self.client.has_key() {
                    v.extend([
                        run("Дивлюсь", "/list watching", "/list watching"),
                        run("Мої списки", "/list", "/list"),
                        run("Історія переглядів", "/history", "/history"),
                        run("Сповіщення", "/notif", "/notif"),
                        run("Акаунт", "/whoami", "/whoami"),
                        run("Вийти", "/logout", "/logout"),
                    ]);
                } else {
                    v.push(run("Увійти", "/login", "/login"));
                }
                v.extend([
                    run("Довідка", "/help", "/help"),
                    run("Очистити історію", "/clear", "/clear"),
                    run("Вихід", "/quit", "/quit"),
                ]);
                v
            }
            PMode::Search => p
                .results
                .iter()
                .filter_map(|a| {
                    let hint = [a.format.as_deref().map(str::to_uppercase), a.year.map(|y| y.to_string())]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    Some(PItem { label: a.titles.main().into(), hint, action: PAction::Run(format!("/view {}", a.reference()?)) })
                })
                .collect(),
            PMode::Releases => self
                .session
                .as_ref()
                .and_then(|s| s.player.as_ref())
                .map(|pl| {
                    pl.releases
                        .iter()
                        .enumerate()
                        .map(|(i, r)| PItem {
                            label: r.team_names(),
                            hint: format!(
                                "{} · до {} еп.",
                                r.kind_label(),
                                r.last_episode.map(fmt_num).unwrap_or_else(|| "?".into())
                            ),
                            action: PAction::Episodes(i),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            PMode::Episodes(i) => {
                let s = self.session.as_ref();
                let watched = s.and_then(|s| s.entry.clone().flatten()).and_then(|e| e.episodes).unwrap_or(0.0);
                s.and_then(|s| s.player.as_ref())
                    .and_then(|pl| pl.releases.get(*i))
                    .map(|r| {
                        r.episodes
                            .iter()
                            .map(|e| PItem {
                                label: format!("{} {} еп.", if e.number <= watched { "✓" } else { " " }, e.label()),
                                hint: e.sources.iter().map(|s| s.provider.as_str()).collect::<Vec<_>>().join(" "),
                                action: PAction::Run(format!("/watch {} {}", fmt_num(e.number), r.id)),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };
        if p.mode == PMode::Search || p.query.is_empty() {
            return items;
        }
        let q = p.query.to_lowercase();
        items.into_iter().filter(|it| fuzzy(&q, &it.label.to_lowercase()) || fuzzy(&q, &it.hint.to_lowercase())).collect()
    }

    fn palette_key(&mut self, k: KeyEvent) {
        let items_len = self.palette_items().len();
        let Some(p) = &mut self.palette else { return };
        match k.code {
            KeyCode::Esc => {
                if p.mode == PMode::Actions {
                    self.palette = None;
                } else {
                    p.mode = match p.mode {
                        PMode::Episodes(_) => PMode::Releases,
                        _ => PMode::Actions,
                    };
                    p.query.clear();
                    p.sel = 0;
                }
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => self.palette = None,
            KeyCode::Up => p.sel = p.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => p.sel = (p.sel + 1).min(items_len.saturating_sub(1)),
            KeyCode::PageUp => p.sel = p.sel.saturating_sub(10),
            KeyCode::PageDown => p.sel = (p.sel + 10).min(items_len.saturating_sub(1)),
            KeyCode::Backspace => {
                p.query.pop();
                p.sel = 0;
                p.typed_at = Instant::now();
            }
            KeyCode::Char(c) => {
                p.query.push(c);
                p.sel = 0;
                p.typed_at = Instant::now();
            }
            KeyCode::Enter => {
                let items = self.palette_items();
                let Some(p) = &mut self.palette else { return };
                let Some(item) = items.get(p.sel) else { return };
                match item.action.clone() {
                    PAction::Run(cmd) => {
                        self.palette = None;
                        self.submit(cmd);
                    }
                    PAction::Recent => {
                        p.mode = PMode::Recent;
                        p.query.clear();
                        p.sel = 0;
                        if self.client.has_key() {
                            self.spawn_raw(|c| Msg::History(c.history(1, 50).map(|p| p.items)));
                        }
                    }
                    PAction::Search => {
                        p.mode = PMode::Search;
                        p.query.clear();
                        p.results.clear();
                        p.searched.clear();
                        p.sel = 0;
                    }
                    PAction::Releases => {
                        p.mode = PMode::Releases;
                        p.query.clear();
                        p.sel = 0;
                        if let Some(s) = &self.session {
                            if s.player.is_none() {
                                let r = s.r.clone();
                                self.spawn_raw(move |c| Msg::Player(r.clone(), c.player(&r)));
                            }
                        }
                    }
                    PAction::Episodes(i) => {
                        p.mode = PMode::Episodes(i);
                        p.query.clear();
                        // Start at the first unwatched episode.
                        let watched = self.session.as_ref().and_then(|s| s.entry.clone().flatten()).and_then(|e| e.episodes).unwrap_or(0.0);
                        p.sel = self
                            .session
                            .as_ref()
                            .and_then(|s| s.player.as_ref())
                            .and_then(|pl| pl.releases.get(i))
                            .and_then(|r| r.episodes.iter().position(|e| e.number > watched))
                            .unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }

    /// Live search in the palette, debounced.
    fn palette_search_tick(&mut self) {
        let Some(p) = &mut self.palette else { return };
        if p.mode != PMode::Search || p.query == p.searched || p.typed_at.elapsed() < Duration::from_millis(250) {
            return;
        }
        p.searched = p.query.clone();
        p.generation += 1;
        let generation = p.generation;
        let q = p.query.trim().to_string();
        if q.is_empty() {
            p.results.clear();
            return;
        }
        self.spawn_raw(move |c| Msg::Found(generation, c.search(&cli::search_query(&q, 30, 1)).map(|p| p.items)));
    }

    // ---------- drawing ----------

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        if self.page.is_some() {
            let main = Rect { height: area.height.saturating_sub(1), ..area };
            self.draw_page(f, main);
            self.draw_status(f, Rect { y: area.y + area.height.saturating_sub(1), height: 1, ..area });
            if self.palette.is_some() {
                self.draw_palette(f, area);
            }
            return;
        }
        let bottom_h = 1 + 1 + 3 + 1; // gap, session, input with rules, status
        let transcript = Rect { height: area.height.saturating_sub(bottom_h), ..area };
        let y = transcript.y + transcript.height;
        let session_area = Rect { y: y + 1, height: 1, ..area };
        let input_area = Rect { y: y + 2, height: 3, ..area };
        let status_area = Rect { y: y + 5, height: 1, ..area };

        self.draw_transcript(f, transcript);
        self.draw_session(f, session_area);
        self.draw_input(f, input_area);
        self.draw_status(f, status_area);

        let comps = self.completions();
        if !comps.is_empty() && self.palette.is_none() {
            draw_completions(f, transcript, &comps, self.ac_sel.min(comps.len() - 1));
        }
        if self.palette.is_some() {
            self.draw_palette(f, area);
        }
    }

    fn draw_transcript(&mut self, f: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(3).max(10);
        let spinner = SPINNER[self.tick % SPINNER.len()];
        let mut all: Vec<Line> = vec![];
        let mut owner: Vec<Option<usize>> = vec![];
        // Rows drawn on the plain terminal background: margins and headers of expanded blocks.
        let mut bare: Vec<usize> = vec![];
        let mut ranges: Vec<(usize, usize)> = vec![];
        let mut zebra = 0;
        let mut shade: Vec<usize> = vec![0; self.entries.len()];
        // Runs of the same command show as one block: the last one, with a count.
        let folded: Vec<bool> = (0..self.entries.len()).map(|i| folded_into_next(&self.entries, i)).collect();
        let mut run: Vec<(usize, Option<DateTime<Utc>>)> = vec![(1, None); self.entries.len()];
        for i in 1..self.entries.len() {
            if folded[i - 1] {
                let (n, first) = run[i - 1];
                run[i] = (n + 1, first.or(Some(self.entries[i - 1].at)));
            }
        }
        for (i, e) in self.entries.iter_mut().enumerate() {
            let start = all.len();
            if folded[i] {
                ranges.push((start, start));
                continue;
            }
            if e.cache.as_ref().is_none_or(|(w, _)| *w != width) {
                e.cache = Some((width, render_body(e, width)));
            }
            let body = &e.cache.as_ref().unwrap().1;
            if e.cmd.is_empty() {
                all.push(Line::raw(""));
                all.extend(body.iter().cloned());
                owner.resize(all.len(), None);
                ranges.push((start, all.len()));
                continue;
            }
            shade[i] = zebra;
            zebra += 1;
            let collapsed = e.collapsed && !e.pending;
            if !collapsed {
                bare.extend([all.len(), all.len() + 1]);
                all.push(Line::raw(""));
            }
            all.push(header_line(e, width, collapsed, run[i]));
            if !collapsed {
                all.push(Line::raw(""));
                all.extend(body.iter().cloned());
                if e.pending {
                    all.push(Line::from(vec![Span::raw("  "), Span::styled(spinner, Style::new().fg(ACCENT))]));
                }
                all.push(Line::raw(""));
            }
            owner.resize(all.len(), Some(i));
            ranges.push((start, all.len()));
        }

        let h = area.height as usize;
        self.total_lines = all.len().saturating_sub(h);
        if self.reveal_focus {
            self.reveal_focus = false;
            if let Some((start, end)) = self.focus.and_then(|i| ranges.get(i).copied()) {
                let win_end = all.len() - self.scroll.min(all.len());
                let win_start = win_end.saturating_sub(h);
                let new_end = if start < win_start {
                    start + h
                } else if end > win_end {
                    end.min(start + h)
                } else {
                    win_end
                };
                self.scroll = all.len().saturating_sub(new_end.min(all.len()));
            }
        }
        self.scroll = self.scroll.min(self.total_lines);
        let end = all.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(h);
        let visible: Vec<Line> = all.drain(start..end).collect();
        self.row_owner = owner[start..end].to_vec();
        self.transcript_y = area.y;

        // Column 0 is for the focus bar; blocks start at column 1.
        for (row, o) in self.row_owner.iter().enumerate() {
            let Some(i) = o else { continue };
            if bare.binary_search(&(start + row)).is_ok() {
                continue;
            }
            let bg = if shade[*i] % 2 == 0 { SURFACE } else { SURFACE_ALT };
            ui::fill(f, Rect { x: area.x + 1, y: area.y + row as u16, width: area.width.saturating_sub(1), height: 1 }, bg);
        }
        f.render_widget(Paragraph::new(visible), Rect { x: area.x + 2, width: area.width.saturating_sub(2), ..area });
        if let Some(fi) = self.focus {
            for (row, o) in self.row_owner.iter().enumerate() {
                if *o == Some(fi) {
                    ui::focus_bar(f, area.x, area.y + row as u16, 1);
                }
            }
        }
    }

    fn draw_session(&self, f: &mut Frame, area: Rect) {
        let line = match &self.session {
            Some(s) => {
                let mut spans = vec![Span::styled(" ● ", Style::new().fg(ACCENT)), Span::styled(s.title.clone(), ui::bold())];
                let team = s.team.as_ref().and_then(|t| {
                    s.player.as_ref().and_then(|p| p.releases.iter().find(|r| &r.id == t)).map(|r| r.team_names()).or(Some(t.clone()))
                });
                if let Some(t) = team {
                    spans.push(Span::styled(format!("  ·  {t}"), ui::muted()));
                }
                if let Some(Some(e)) = &s.entry {
                    let total = s.episodes.total.filter(|&t| t > 0).map(|t| t.to_string()).unwrap_or_else(|| "?".into());
                    spans.push(Span::styled("  ·  ", ui::muted()));
                    spans.push(Span::styled(
                        format!("{} {}/{total}", list_status_label(&e.status), e.episodes.map(fmt_num).unwrap_or_else(|| "0".into())),
                        Style::new().fg(GREEN),
                    ));
                }
                if let Some(n) = s.last_ep {
                    spans.push(Span::styled("  ·  ", ui::muted()));
                    spans.push(Span::styled(format!("відкрито {} еп.", fmt_num(n)), Style::new().fg(YELLOW)));
                }
                Line::from(spans)
            }
            None => Line::styled(" ○ немає відкритого аніме", Style::new().fg(FAINT)),
        };
        f.render_widget(line, area);
    }

    fn draw_input(&self, f: &mut Frame, area: Rect) {
        let rule = if self.focus.is_some() { FAINT } else { MUTED };
        f.render_widget(Block::new().borders(Borders::TOP | Borders::BOTTOM).border_style(Style::new().fg(rule)), area);
        let area = Rect { y: area.y + 1, height: 1, ..area };
        let prompt = Span::styled(" › ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD));
        let avail = area.width.saturating_sub(4) as usize;
        let before: String = self.input.chars().take(self.cursor).collect();
        let before_w = before.width();
        // Scroll horizontally so the cursor stays visible.
        let skip_w = before_w.saturating_sub(avail);
        let mut shown = String::new();
        let mut w = 0;
        for c in self.input.chars() {
            let cw = c.width().unwrap_or(0);
            w += cw;
            if w > skip_w {
                shown.push(c);
            }
        }
        let text = if self.input.is_empty() {
            Span::styled("/команда", Style::new().fg(FAINT))
        } else {
            Span::raw(shown)
        };
        f.render_widget(Line::from(vec![prompt, text]), area);
        if self.palette.is_none() && self.focus.is_none() {
            f.set_cursor_position((area.x + 3 + (before_w - skip_w) as u16, area.y));
        }
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        let mut left = vec![Span::raw(" ")];
        if self.loading > 0 {
            left.push(Span::styled(format!("{} ", SPINNER[self.tick % SPINNER.len()]), Style::new().fg(ACCENT)));
        }
        if self.scroll > 0 {
            left.push(Span::styled(format!("↓ ще {} рядків  ", self.scroll), Style::new().fg(YELLOW)));
            left.extend(ui::hint("Esc", "донизу"));
        }
        f.render_widget(Line::from(left), area);
        let keys: Vec<(&str, &str)> = if self.palette.is_some() {
            vec![]
        } else if self.page.is_some() {
            self.page_hints()
        } else if self.focus.is_some() {
            vec![("⏎", "згорнути"), ("Ctrl+↑↓", "блоки"), ("Esc", "назад")]
        } else {
            vec![("Ctrl+P", "меню"), ("Ctrl+H", "нещодавні"), ("Ctrl+↑", "блоки"), ("Ctrl+O", "згорнути")]
        };
        let mut right = ui::hints(&keys);
        if self.page.is_none() && self.focus.is_none() && self.palette.is_none() {
            right.push(Span::styled(format!("   {}", self.user.clone().unwrap_or_else(|| "не увійшли".into())), ui::muted()));
        }
        right.push(Span::raw(" "));
        let rw = ui::width(&right);
        if rw <= area.width {
            f.render_widget(Line::from(right), Rect { x: area.x + area.width - rw, width: rw, ..area });
        }
    }

    fn draw_palette(&self, f: &mut Frame, area: Rect) {
        let Some(p) = &self.palette else { return };
        let title = match &p.mode {
            PMode::Actions => "Меню".to_string(),
            PMode::Recent => "Нещодавно переглянуті".to_string(),
            PMode::Search => "Пошук аніме".to_string(),
            PMode::Releases => "Команда".to_string(),
            PMode::Episodes(i) => self
                .session
                .as_ref()
                .and_then(|s| s.player.as_ref())
                .and_then(|pl| pl.releases.get(*i))
                .map(|r| format!("Серії · {} · {}", r.team_names(), r.kind_label()))
                .unwrap_or_default(),
        };
        let empty = match p.mode {
            PMode::Search if p.query.is_empty() => "почніть вводити назву",
            PMode::Recent if self.loading > 0 => "завантаження…",
            PMode::Recent => "ще нічого не дивились: /watch або /play",
            PMode::Search if p.query != p.searched || self.loading > 0 => "шукаю…",
            PMode::Releases if self.session.as_ref().is_some_and(|s| s.player.is_none()) => "завантаження…",
            _ => "нічого немає",
        };
        let placeholder = if p.mode == PMode::Search { "назва українською, англійською чи оригінальною" } else { "фільтр" };
        let mut footer = vec![("↑↓", "вибір"), ("⏎", "обрати")];
        footer.push(("Esc", if p.mode == PMode::Actions { "закрити" } else { "назад" }));
        let items = self.palette_items().into_iter().map(|it| ui::Item::new(it.label, it.hint)).collect();
        let overlay = ui::Overlay {
            title,
            aside: String::new(),
            filter: Some((&p.query, placeholder)),
            items,
            sel: p.sel,
            empty: empty.into(),
            footer,
            width: 76,
        };
        let layout = overlay.draw(f, area);
        if let Some(c) = layout.cursor {
            f.set_cursor_position(c);
        }
    }
}

fn draw_completions(f: &mut Frame, above: Rect, comps: &[&Spec], sel: usize) {
    let h = (comps.len() as u16).min(8);
    let w = 76.min(above.width.saturating_sub(4));
    let rect = Rect { x: above.x + 1, y: above.y + above.height.saturating_sub(h + 1), width: w, height: h + 1 };
    f.render_widget(Clear, rect);
    ui::fill(f, rect, ui::OVERLAY);
    let items: Vec<ui::Item> = comps
        .iter()
        .map(|c| ui::Item::new(format!("/{:<10} {}", c.name, c.args), c.help))
        .collect();
    ui::rows(f, Rect { x: rect.x + 1, y: rect.y, width: rect.width - 2, height: h }, &items, sel, ui::OVERLAY);
    let hints = ui::hints(&[("Tab", "доповнити"), ("⏎", "виконати")]);
    f.render_widget(Line::from(hints), Rect { x: rect.x + 2, y: rect.y + h, width: rect.width - 2, height: 1 });
}

fn fuzzy(q: &str, s: &str) -> bool {
    let mut it = s.chars();
    q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|x| x == c))
}

// ---------- transcript rendering ----------

fn tone_style(t: Tone) -> Style {
    let s = Style::new();
    match t {
        Tone::Plain => s,
        Tone::Bold => s.add_modifier(Modifier::BOLD),
        Tone::Dim => s.fg(MUTED),
        Tone::Accent => s.fg(ACCENT),
        Tone::Green => s.fg(GREEN),
        Tone::Yellow => s.fg(YELLOW),
        Tone::Red => s.fg(RED),
        // Old saved transcripts may still carry these; show them neutral.
        Tone::Cyan | Tone::Magenta => s,
    }
}

/// Entry `i` is a command repeated right after by the same command.
fn folded_into_next(entries: &[Entry], i: usize) -> bool {
    let Some(e) = entries.get(i) else { return false };
    !e.cmd.is_empty() && entries.get(i + 1).is_some_and(|n| n.cmd == e.cmd)
}

/// `run`: how many times in a row the command ran, and when the first one did.
fn header_line(e: &Entry, width: u16, collapsed: bool, run: (usize, Option<DateTime<Utc>>)) -> Line<'static> {
    let hm = |t: DateTime<Utc>| t.with_timezone(&Local).format("%H:%M").to_string();
    let time = match run.1.map(hm) {
        Some(first) if first != hm(e.at) => format!("{first}–{}", hm(e.at)),
        _ => hm(e.at),
    };
    let arrow = if collapsed { "▸ " } else { "▾ " };
    let mut spans = vec![
        Span::styled(arrow, Style::new().fg(FAINT)),
        Span::styled(e.cmd.clone(), ui::bold()),
    ];
    let mut used = 2 + e.cmd.width();
    if run.0 > 1 {
        let count = format!("  ×{}", run.0);
        used += count.width();
        spans.push(Span::styled(count, ui::muted()));
    }
    let right = time.width() + 2;
    if collapsed {
        // A peek at the output: its first non-empty line.
        let peek = e.lines.iter().map(SLine::text).find(|t| !t.trim().is_empty()).unwrap_or_default();
        let room = (width as usize).saturating_sub(used + right + 4);
        if room > 3 && !peek.is_empty() {
            let peek = crate::style::truncate(peek.trim(), room);
            used += 3 + peek.width();
            spans.push(Span::styled(format!("   {peek}"), ui::muted()));
        }
    }
    spans.push(Span::raw(" ".repeat((width as usize).saturating_sub(used + right))));
    spans.push(Span::styled(time, Style::new().fg(FAINT)));
    Line::from(spans)
}

fn render_body(e: &Entry, width: u16) -> Vec<Line<'static>> {
    let indent = if e.cmd.is_empty() { "" } else { "    " };
    let mut out = vec![];
    for l in &e.lines {
        wrap_line(l, width as usize, indent, &mut out);
    }
    out
}

fn wrap_line(l: &SLine, width: usize, indent: &str, out: &mut Vec<Line<'static>>) {
    let avail = width.saturating_sub(indent.width()).max(4);
    let mut cur: Vec<Span<'static>> = vec![Span::raw(indent.to_string())];
    if l.nowrap {
        let total: usize = l.segs.iter().map(|s| s.0.width()).sum();
        let budget = if total > avail { avail - 1 } else { avail };
        let mut used = 0;
        'outer: for Seg(s, t) in &l.segs {
            let mut buf = String::new();
            for c in s.chars() {
                let cw = c.width().unwrap_or(0);
                if used + cw > budget {
                    cur.push(Span::styled(buf, tone_style(*t)));
                    cur.push(Span::styled("…", Style::new().fg(Color::DarkGray)));
                    break 'outer;
                }
                buf.push(c);
                used += cw;
            }
            cur.push(Span::styled(buf, tone_style(*t)));
        }
        out.push(Line::from(cur));
        return;
    }
    let mut w = 0;
    for Seg(s, t) in &l.segs {
        let style = tone_style(*t);
        for word in s.split_inclusive(' ') {
            let ww = word.width();
            if w + ww > avail && w > 0 {
                out.push(Line::from(std::mem::replace(&mut cur, vec![Span::raw(indent.to_string())])));
                w = 0;
                if word.trim().is_empty() {
                    continue;
                }
            }
            if ww > avail {
                for c in word.chars() {
                    let cw = c.width().unwrap_or(0);
                    if w + cw > avail {
                        out.push(Line::from(std::mem::replace(&mut cur, vec![Span::raw(indent.to_string())])));
                        w = 0;
                    }
                    cur.push(Span::styled(c.to_string(), style));
                    w += cw;
                }
            } else {
                cur.push(Span::styled(word.to_string(), style));
                w += ww;
            }
        }
    }
    out.push(Line::from(cur));
}

// ---------- command execution (worker threads) ----------

/// Everything a command may read, copied at submit time.
struct Snap {
    client: Client,
    session: Option<Session>,
    anime_refs: Vec<String>,
    release_refs: Vec<String>,
    recent: Vec<Recent>,
    tx: Sender<Msg>,
    id: u64,
}

impl Snap {
    fn report(&self, lines: Lines) {
        let _ = self.tx.send(Msg::Append(self.id, lines));
    }

    fn session(&self) -> Result<&Session> {
        self.session.as_ref().context("немає відкритого аніме: /show <назва> або Ctrl+P → Пошук")
    }

    fn need_key(&self) -> Result<()> {
        if !self.client.has_key() {
            bail!("потрібен вхід: /login");
        }
        Ok(())
    }

    fn player(&self, out: &mut Outcome) -> Result<Player> {
        let s = self.session()?;
        if let Some(p) = &s.player {
            return Ok(p.clone());
        }
        let p = self.client.player(&s.r)?;
        out.player = Some((s.r.clone(), p.clone()));
        Ok(p)
    }

    /// `#N` from the last list, the open session for an empty arg, or any ref/title.
    fn anime(&self, arg: &str) -> Result<String> {
        if arg.is_empty() {
            return Ok(self.session()?.r.clone());
        }
        if let Some(n) = arg.strip_prefix('#') {
            let n: usize = n.parse().context("після # має бути номер")?;
            return self.anime_refs.get(n.wrapping_sub(1)).cloned().with_context(|| format!("в останньому списку немає #{n}"));
        }
        Ok(cli::resolve(&self.client, arg)?.0)
    }

    fn release_filter(&self, arg: &str) -> Result<String> {
        if let Some(n) = arg.strip_prefix('#') {
            let n: usize = n.parse().context("після # має бути номер")?;
            return self.release_refs.get(n.wrapping_sub(1)).cloned().with_context(|| format!("немає релізу #{n}: спершу /eps"));
        }
        Ok(arg.to_string())
    }

    fn save_entry(&self, out: &mut Outcome, patch: serde_json::Value) -> Result<()> {
        self.need_key()?;
        let s = self.session()?;
        let mut patch = patch;
        if matches!(s.entry, Some(None)) && patch.get("status").is_none() {
            patch["status"] = json!("watching");
        }
        let e = self.client.save_entry(&s.r, &patch)?;
        out.lines.push(SLine::of("✓ ", Tone::Green).push(view::entry_summary(&e, &s.episodes).text(), Tone::Plain));
        out.entry = Some((s.r.clone(), Some(e)));
        Ok(())
    }
}

fn open_anime(snap: &Snap, r: &str, detail: Option<AnimeDetail>, page: bool) -> Result<Outcome> {
    let c = &snap.client;
    let d = match detail {
        Some(d) => d,
        None => c.anime(r)?,
    };
    let r = d.ids.mikai.map(|i| i.to_string()).unwrap_or_else(|| r.to_string());
    let entry = if c.has_key() { Some(cli::entry_of(c, &r)?) } else { None };
    let prev = snap.session.as_ref().filter(|s| s.r == r);
    let keep_team = prev.and_then(|s| s.team.clone());
    let last_ep = prev.and_then(|s| s.last_ep);
    let (lines, player) = if page {
        let player = match prev.and_then(|s| s.player.clone()) {
            Some(p) => p,
            None => c.player(&r)?,
        };
        (vec![SLine::of("Сторінка перегляду: ", Tone::Dim).push(d.titles.main(), Tone::Bold)], Some(player))
    } else {
        let mut lines = view::anime_card(&d, entry.as_ref().map(|e| e.as_ref()), true);
        lines.push(dim("/view — сторінка перегляду · /eps #N — серії · /watch N — дивитись"));
        (lines, prev.and_then(|s| s.player.clone()))
    };
    Ok(Outcome {
        lines,
        page: page.then(|| d.clone()),
        release_refs: Some(d.releases.iter().map(|x| x.id.clone()).collect()),
        open: Some(Session {
            r: r.clone(),
            title: d.titles.main().to_string(),
            url: d.url(),
            episodes: d.episodes.clone(),
            team: keep_team,
            last_ep,
            player,
            entry,
        }),
        ..Default::default()
    })
}

fn watch(snap: &Snap, n: f64, team: Option<String>, provider: Option<&str>, mpv: bool) -> Result<Outcome> {
    let mut out = Outcome::default();
    let s = snap.session()?;
    let p = snap.player(&mut out)?;
    if p.licensed {
        bail!("аніме ліцензоване в Україні, джерел немає (/site)");
    }
    let filter = |team: Option<String>| cli::Filter { team, kind: None, provider: provider.map(str::to_string), mpv };
    let (rel, ep, src) = match cli::pick(&p.releases, n, &filter(team.clone())) {
        Ok(found) => found,
        // The remembered team may not have this episode yet: fall back to any.
        Err(_) if team.is_some() && provider.is_none() => cli::pick(&p.releases, n, &filter(None))?,
        Err(e) => return Err(e),
    };
    cli::play(&cli::media_title(&s.title, ep, rel), src, mpv)?;
    out.lines.push(
        SLine::of(if mpv { "▶ mpv  " } else { "▶ " }, Tone::Accent)
            .push(format!("{} еп.", ep.label()), Tone::Bold)
            .push(format!("  {} · {} · {}", rel.team_names(), rel.kind_label(), src.provider), Tone::Plain),
    );
    if !mpv {
        out.lines.push(dim(src.embed_url.clone()));
    }
    out.team = Some((s.r.clone(), rel.id.clone()));
    out.last_ep = Some((s.r.clone(), n));
    if snap.client.has_key() {
        let watched = s.entry.clone().flatten().and_then(|e| e.episodes).unwrap_or(0.0);
        if n.floor() > watched {
            out.lines.push(dim(format!("/ep {} — позначити переглянутою", fmt_num(n.floor()))));
        }
    }
    Ok(out)
}

fn status_arg(arg: &str) -> Result<&'static str> {
    let a = arg.to_lowercase();
    LIST_STATUSES
        .iter()
        .find(|s| !a.is_empty() && (s.starts_with(&a) || list_status_label(s).starts_with(&a)))
        .copied()
        .with_context(|| format!("невідомий статус «{arg}»: {}", LIST_STATUSES.join(", ")))
}

fn exec(name: &str, args: &str, snap: &Snap) -> Result<Outcome> {
    let c = &snap.client;
    let words: Vec<&str> = args.split_whitespace().collect();
    let mut out = Outcome::default();
    match name {
        "search" => {
            if args.is_empty() {
                bail!("що шукати? /search <назва>");
            }
            let page = c.search(&cli::search_query(args, 20, 1))?;
            out.lines = view::anime_rows(&page.items, true);
            out.lines.extend(view::page_footer(page.page, page.pages, page.total));
            if !page.items.is_empty() {
                out.lines.push(dim("/show #N — відкрити"));
            }
            out.anime_refs = Some(page.items);
        }
        "show" => {
            if args.is_empty() {
                bail!("яке аніме? /show <назва|id|#N>");
            }
            return open_anime(snap, &snap.anime(args)?, None, false);
        }
        "random" => {
            let d = c.random()?;
            let r = d.ids.mikai.map(|i| i.to_string()).unwrap_or_default();
            return open_anime(snap, &r, Some(d), false);
        }
        "eps" => {
            let s = snap.session()?;
            let p = snap.player(&mut out)?;
            if p.licensed {
                bail!("аніме ліцензоване в Україні, джерел немає (/site)");
            }
            if args.is_empty() {
                out.lines = view::releases(&p.releases, true);
                out.lines.push(dim("/eps #N — серії релізу"));
                out.release_refs = Some(p.releases.iter().map(|r| r.id.clone()).collect());
            } else {
                let filter = snap.release_filter(args)?;
                let rel = p.releases.iter().find(|r| r.matches(&filter)).with_context(|| format!("немає релізу «{args}»"))?;
                let watched = s.entry.clone().flatten().and_then(|e| e.episodes).unwrap_or(0.0);
                out.lines = view::episodes(rel, watched, None);
                out.lines.push(dim("/watch N — дивитись"));
                out.team = Some((s.r.clone(), rel.id.clone()));
            }
        }
        "watch" | "play" => {
            let mpv = name == "play" || words.iter().any(|w| *w == "--mpv" || *w == "-m");
            let pos: Vec<&str> = words.iter().copied().filter(|w| !w.starts_with('-')).collect();
            let n: f64 = pos
                .first()
                .with_context(|| format!("яку серію? /{name} <N> [команда] [плеєр]"))?
                .replace(',', ".")
                .parse()
                .context("номер серії має бути числом")?;
            let team = match pos.get(1) {
                Some(t) => Some(snap.release_filter(t)?),
                None => snap.session()?.team.clone(),
            };
            return watch(snap, n, team, pos.get(2).copied(), mpv);
        }
        "next" => {
            let s = snap.session()?;
            let watched = s.entry.clone().flatten().and_then(|e| e.episodes).unwrap_or(0.0);
            let n = s.last_ep.unwrap_or(0.0).max(watched).floor() + 1.0;
            let mpv = words.iter().any(|w| *w == "--mpv" || *w == "-m");
            return watch(snap, n, s.team.clone(), None, mpv);
        }
        "view" | "resume" => {
            let r = if args.is_empty() && name == "resume" {
                snap.recent.first().map(|x| x.r.clone()).context("ще нічого не дивились")?
            } else {
                snap.anime(args)?
            };
            // Both pick up where the title was left: team and next episode.
            let mut out = open_anime(snap, &r, None, true)?;
            if let (Some(s), Some(rec)) = (out.open.as_mut(), snap.recent.iter().find(|x| x.r == r)) {
                s.last_ep = Some(s.last_ep.unwrap_or(0.0).max(rec.ep));
                if s.team.is_none() {
                    s.team = rec.team.clone();
                }
                out.lines = vec![
                    SLine::of("Продовжити: ", Tone::Dim)
                        .push(s.title.clone(), Tone::Bold)
                        .push(format!("  після {} еп.", fmt_num(rec.ep)), Tone::Dim),
                ];
            }
            return Ok(out);
        }
        "ep" => {
            let n: i64 = args.parse().context("/ep <номер серії>")?;
            snap.save_entry(&mut out, json!({ "episodes": n }))?;
        }
        "status" => {
            let st = status_arg(args)?;
            snap.save_entry(&mut out, json!({ "status": st }))?;
        }
        "score" => {
            let v: f64 = args.replace(',', ".").parse().context("/score <0.5–10>, 0 — прибрати")?;
            snap.save_entry(&mut out, json!({ "score": if v == 0.0 { serde_json::Value::Null } else { json!(v) } }))?;
        }
        "rm" => {
            snap.need_key()?;
            let s = snap.session()?;
            c.delete_entry(&s.r)?;
            out.lines.push(plain("Прибрано зі списків"));
            out.entry = Some((s.r.clone(), None));
        }
        "site" => {
            let s = snap.session()?;
            let url = match &s.url {
                Some(u) => u.clone(),
                None => c.anime(&s.r)?.url().context("у аніме немає сторінки")?,
            };
            open::that_detached(&url).context("не вдалося відкрити браузер")?;
            out.lines.push(dim(url));
        }
        "schedule" => {
            let day = (!args.is_empty()).then(|| view::weekday_key(args));
            let (lines, refs) = view::schedule(c.schedule()?, day.as_deref(), true);
            out.lines = lines;
            out.anime_refs = Some(refs);
        }
        "updates" => {
            let n: u32 = if args.is_empty() { 20 } else { args.parse().context("/updates [кількість]")? };
            let p = c.updates(1, n.min(100))?;
            out.lines = view::updates(&p.items, true);
            out.anime_refs = Some(p.items.into_iter().map(|u| u.anime).collect());
        }
        "list" => {
            snap.need_key()?;
            let st = if args.is_empty() { None } else { Some(status_arg(args)?) };
            let p = c.my_list(st, 1, 100)?;
            out.lines = view::list_entries(&p.items, true);
            out.lines.extend(view::page_footer(p.page, p.pages, p.total));
            out.anime_refs = Some(p.items.into_iter().map(|e| e.anime).collect());
        }
        "history" => {
            snap.need_key()?;
            let p = c.history(1, 30)?;
            out.lines = view::history(&p.items, true);
            out.anime_refs = Some(p.items.iter().map(view::anime_of).collect());
        }
        "notif" => {
            snap.need_key()?;
            if args == "read" {
                c.mark_all_read()?;
                out.lines.push(plain("Усі сповіщення прочитано"));
            } else {
                let filter = if args == "all" { "all" } else { "unread" };
                out.lines = view::notifications(&c.notifications(filter, 1, 30)?.items);
            }
        }
        "translit" => {
            if args.is_empty() {
                bail!("/translit <текст>");
            }
            let r = c.translit(args, None)?;
            out.lines.push(plain(r.get("text").and_then(|v| v.as_str()).map(str::to_string).unwrap_or_else(|| r.to_string())));
        }
        "whoami" => out.lines = view::me(&c.me()?),
        "login" => {
            let mut config = Config::load()?;
            let mut client = c.clone();
            if let Some(key) = words.first().filter(|k| k.starts_with("mk_")) {
                let (user, level) = cli::save_key(&mut client, &mut config, key.to_string())?;
                out.lines.push(SLine::of("✓ ", Tone::Green).push(format!("Увійшли як {user} (рівень {})", level.as_deref().unwrap_or("?")), Tone::Plain));
                out.auth = Some(Some((key.to_string(), user)));
            } else {
                let Some(client_id) = config.client_id() else {
                    out.lines = vec![
                        plain("Вхід за посиланням ще не налаштовано (немає clientId)."),
                        plain("Створіть ключ на ").push("https://mikai.me/user/developers", Tone::Bold).push(" і введіть:", Tone::Plain),
                        SLine::of("/login mk_…", Tone::Accent),
                    ];
                    return Ok(out);
                };
                let token = cli::device_login(c, &client_id, "write", true, |url| {
                    snap.report(vec![
                        plain("Підтвердіть вхід у браузері:"),
                        SLine::of(url, Tone::Bold),
                        dim("чекаю на підтвердження…"),
                    ]);
                })?;
                let user = token.user.as_ref().map(User::label).unwrap_or_else(|| "?".into());
                config.key = Some(token.key.clone());
                config.level = token.level.clone();
                config.user = Some(user.clone());
                config.client_id = Some(client_id);
                config.save()?;
                out.lines.push(SLine::of("✓ ", Tone::Green).push(format!("Увійшли як {user} (рівень {})", token.level.as_deref().unwrap_or("?")), Tone::Plain));
                out.auth = Some(Some((token.key, user)));
            }
        }
        "logout" => {
            let mut config = Config::load()?;
            if c.has_key() {
                if let Err(e) = c.delete_key() {
                    out.lines.push(dim(format!("Ключ на сервері не видалено: {e:#}")));
                }
            }
            config.key = None;
            config.level = None;
            config.user = None;
            config.save()?;
            out.lines.push(plain("Вийшли"));
            out.auth = Some(None);
        }
        other => bail!("команда /{other} тут не підтримується"),
    }
    Ok(out)
}
