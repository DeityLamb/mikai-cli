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
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
            MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
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

mod keys;
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
    Spec { name: "view", aliases: &["v", "open"], args: "<аніме|#N>", help: "сторінка тайтлу: серії, плеєр, список" },
    Spec { name: "random", aliases: &[], args: "", help: "випадкове аніме" },
    Spec { name: "schedule", aliases: &["sch"], args: "[день]", help: "розклад виходу серій" },
    Spec { name: "updates", aliases: &["new"], args: "[кількість]", help: "щойно додані серії" },
    Spec { name: "list", aliases: &["ls"], args: "[статус]", help: "мої списки" },
    Spec { name: "login", aliases: &[], args: "[mk_…]", help: "увійти (за посиланням або ключем)" },
    Spec { name: "logout", aliases: &[], args: "", help: "вийти" },
    Spec { name: "clear", aliases: &[], args: "[recent|all]", help: "стерти історію; recent — нещодавні, all — усе" },
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
    out.push(dim("#N — рядок останнього списку: /view #2. Аніме — назва, id, mal:21 або посилання."));
    out.push(blank());
    out.push(SLine::of("Клавіші", Tone::Bold));
    for (k, d) in [
        ("Ctrl+P", "пошук аніме"),
        ("Ctrl+Shift+P", "меню (або > у пошуку)"),
        ("Ctrl+H", "нещодавно переглянуті — продовжити з наступної серії"),
        ("на сторінці тайтлу", "c команда/плеєр · r пов'язані · ⏎ дивитись · w переглянуто · s список"),
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
        plain("Знайдіть тайтл: ").push("Ctrl+P", Tone::Accent).push(" або ", Tone::Plain).push("/search назва", Tone::Accent).push(". Меню — ", Tone::Plain).push("Ctrl+Shift+P", Tone::Accent).push(".", Tone::Plain),
        plain("Дивіться на сторінці тайтлу: ").push("/view назва", Tone::Accent).push(" — серії, озвучення, mpv чи браузер.", Tone::Plain),
        plain("Продовжити, де зупинились — ").push("Ctrl+H", Tone::Accent).push(". Усі команди — ", Tone::Plain).push("/help", Tone::Accent).push(".", Tone::Plain),
    ]
}

// ---------- state ----------

/// What the watch page knows about its title.
#[derive(Clone)]
struct Session {
    r: String,
    title: String,
    url: Option<String>,
    /// Last release used, as a release id.
    team: Option<String>,
    last_ep: Option<f64>,
    player: Option<Player>,
    /// `None` — unknown (not logged in), `Some(None)` — in no list.
    entry: Option<Option<ListEntry>>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    /// Empty for notes and separators that are not commands.
    cmd: String,
    at: DateTime<Utc>,
    lines: Lines,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    collapsed: bool,
    /// A page action (played, marked watched…): one line with its time.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    action: bool,
    #[serde(skip)]
    pending: bool,
    #[serde(skip)]
    id: u64,
    #[serde(skip)]
    cache: Option<(u16, Vec<Line<'static>>)>,
}

/// A title that was watched, for Ctrl+H and the watch page.
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
    /// Targets of `#N` in the last list.
    #[serde(default)]
    anime_refs: Vec<String>,
    #[serde(default)]
    recent: Vec<Recent>,
    /// Recent titles from before this moment were cleared on purpose.
    #[serde(default)]
    recent_cleared: Option<DateTime<Utc>>,
}

fn state_path() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::data_local_dir).map(|d| d.join("mikai").join("session.json"))
}

/// What a finished command or page action changes besides printing.
#[derive(Default)]
struct Outcome {
    lines: Lines,
    anime_refs: Option<Vec<AnimeShort>>,
    /// Open the watch page.
    page: Option<(AnimeDetail, Session)>,
    /// New list entry of a title (`None` — removed from lists).
    entry: Option<(String, Option<ListEntry>)>,
    /// Episode `n` of a title was opened with this release.
    played: Option<Recent>,
    /// `Some(None)` — logged out.
    auth: Option<Option<(String, String)>>,
}

enum Msg {
    Append(u64, Lines),
    Done(u64, Result<Outcome>),
    Found(u64, Result<Vec<AnimeShort>>),
    Suggested(u64, Result<Vec<Suggestion>>),
    History(Result<Vec<serde_json::Value>>),
    Teams(Result<Vec<Team>>),
}

/// What `/view <arg>` would open, shown while typing.
#[derive(Clone)]
struct Suggestion {
    id: String,
    title: String,
    meta: String,
}

impl Suggestion {
    fn from_short(a: &AnimeShort) -> Option<Self> {
        Some(Self { id: a.reference()?, title: a.titles.main().to_string(), meta: suggestion_meta(&a.titles, a.format.as_deref(), a.year) })
    }

    fn from_detail(d: &AnimeDetail) -> Option<Self> {
        Some(Self {
            id: d.ids.mikai?.to_string(),
            title: d.titles.main().to_string(),
            meta: suggestion_meta(&d.titles, d.format.as_deref(), d.year),
        })
    }
}

fn suggestion_meta(t: &Titles, format: Option<&str>, year: Option<i64>) -> String {
    [t.secondary().map(str::to_string), format.map(str::to_uppercase), year.map(|y| y.to_string())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

#[derive(Default)]
struct Suggest {
    /// Argument the shown items belong to.
    shown_for: String,
    /// Argument being typed and when it last changed.
    wanted: String,
    changed: Option<Instant>,
    requested: String,
    generation: u64,
    items: Vec<Suggestion>,
}

#[derive(Clone)]
enum PAction {
    Run(String),
    Recent,
    Search,
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

    anime_refs: Vec<String>,
    recent: Vec<Recent>,
    recent_cleared: Option<DateTime<Utc>>,
    /// Team id → name, for releases known only by id (`t_35:voice`).
    team_names: std::collections::HashMap<i64, String>,

    palette: Option<Palette>,
    suggest: Suggest,
    page: Option<page::Page>,
    /// Pages left by opening a related title; Esc returns to them.
    page_stack: Vec<page::Page>,
    tick: usize,
    loading: usize,
    quit: bool,
}

pub fn run(client: Client, user: Option<String>) -> Result<()> {
    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    // Lets terminals that support it (kitty, Alacritty, WezTerm, foot, Ghostty…)
    // tell Ctrl+Shift+P from Ctrl+P. Others fall back to `>` in the search.
    let enhanced = ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        let _ = execute!(stdout(), PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    }
    let mut app = App::new(client, user);
    let result = app.run(&mut terminal);
    if enhanced {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
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
            anime_refs: saved.anime_refs,
            recent: saved.recent,
            recent_cleared: saved.recent_cleared,
            team_names: Default::default(),
            palette: None,
            suggest: Suggest::default(),
            page: None,
            page_stack: vec![],
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
        // A clear in another window wins over our older records.
        if disk.recent_cleared > self.recent_cleared {
            self.recent_cleared = disk.recent_cleared;
            let cut = self.recent_cleared;
            self.recent.retain(|x| Some(x.at) > cut);
        }
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
            "recent": &self.recent,
            "recent_cleared": &self.recent_cleared,
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
        self.entries.push(Entry { cmd: String::new(), at: Utc::now(), lines, pending: false, collapsed: false, action: false, id, cache: None });
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
            self.suggest_tick();
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

        // On a Ukrainian keyboard the `/` key types `.`.
        let body = text.strip_prefix('/').or_else(|| text.strip_prefix('.')).unwrap_or(&text);
        let (name, args) = body.split_once(char::is_whitespace).unwrap_or((body, ""));
        let args = args.trim().to_string();
        // A command name typed in the wrong layout still counts: `іуфкср` → `search`.
        let spec = find_command(&name.to_lowercase()).or_else(|| find_command(&keys::command_word(&name.to_lowercase())));
        let display = match spec {
            Some(sp) if sp.name != name.to_lowercase() && !sp.aliases.contains(&name.to_lowercase().as_str()) => {
                format!("/{} {args}", sp.name).trim_end().to_string()
            }
            _ => format!("/{}", body.trim()),
        };

        let Some(spec) = spec else {
            let id = self.bump();
            let lines = vec![err(format!("Невідома команда «{name}».")).push("  /help — список, Ctrl+P — меню", Tone::Dim)];
            self.entries.push(Entry { cmd: display, at: Utc::now(), lines, pending: false, collapsed: false, action: false, id, cache: None });
            return;
        };

        // Commands that only touch local state.
        let local: Option<Lines> = match spec.name {
            "quit" => {
                self.quit = true;
                return;
            }
            "clear" => {
                let what = args.to_lowercase();
                if !matches!(what.as_str(), "" | "recent" | "all") {
                    Some(vec![err("/clear, /clear recent або /clear all")])
                } else {
                    if what != "recent" {
                        self.entries.clear();
                        self.focus = None;
                    }
                    if what == "all" {
                        self.inputs.clear();
                    }
                    if what != "" {
                        self.recent.clear();
                        self.recent_cleared = Some(Utc::now());
                    }
                    self.save();
                    if what == "recent" {
                        Some(vec![dim("Нещодавні стерто")])
                    } else {
                        return;
                    }
                }
            }
            "help" => Some(help_lines()),
            _ => None,
        };
        let id = self.bump();
        if let Some(lines) = local {
            self.entries.push(Entry { cmd: display, at: Utc::now(), lines, pending: false, collapsed: false, action: false, id, cache: None });
            return;
        }

        self.entries.push(Entry { cmd: display, at: Utc::now(), lines: vec![], pending: true, collapsed: false, action: false, id, cache: None });
        let snap = self.snap(id);
        let name = spec.name;
        let tx = self.tx.clone();
        self.loading += 1;
        thread::spawn(move || {
            let res = exec(name, &args, &snap);
            let _ = tx.send(Msg::Done(id, res));
        });
    }

    fn snap(&self, id: u64) -> Snap {
        Snap {
            client: self.client.clone(),
            anime_refs: self.anime_refs.clone(),
            recent: self.recent.clone(),
            tx: self.tx.clone(),
            id,
        }
    }

    /// Runs a page action off the UI thread; it shows in the transcript as
    /// one line, first with a spinner, then with the result.
    fn run_action(&mut self, f: impl FnOnce(&Client) -> Result<Outcome> + Send + 'static) -> u64 {
        let id = self.bump();
        self.entries.push(Entry { cmd: String::new(), at: Utc::now(), lines: vec![], pending: true, collapsed: false, action: true, id, cache: None });
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.loading += 1;
        thread::spawn(move || {
            let _ = tx.send(Msg::Done(id, f(&client)));
        });
        id
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
            Msg::Suggested(generation, res) => {
                self.loading = self.loading.saturating_sub(1);
                if generation == self.suggest.generation {
                    self.suggest.items = res.unwrap_or_default();
                    self.suggest.shown_for = self.suggest.requested.clone();
                    self.ac_sel = 0;
                }
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
        }
    }

    fn apply(&mut self, o: Outcome) -> Lines {
        if let Some(v) = o.anime_refs {
            self.anime_refs = v.iter().filter_map(AnimeShort::reference).collect();
        }
        if let Some((detail, s)) = o.page {
            // From one title page to another: keep the way back.
            match self.page.take() {
                Some(cur) if cur.s.r != s.r => {
                    self.page_stack.push(cur);
                    if self.page_stack.len() > 20 {
                        self.page_stack.remove(0);
                    }
                }
                Some(_) => {}
                None => self.page_stack.clear(),
            }
            self.open_page(detail, s);
        }
        if let Some((r, e)) = o.entry {
            if let Some(s) = self.page_session(&r) {
                s.entry = Some(e);
            }
        }
        if let Some(rec) = o.played {
            if let Some(s) = self.page_session(&rec.r) {
                s.last_ep = Some(rec.ep);
                s.team = rec.team.clone();
            }
            self.remember(rec);
        }
        match o.auth {
            Some(Some((key, user))) => {
                self.client.set_key(Some(key));
                self.user = Some(user);
            }
            Some(None) => {
                self.client.set_key(None);
                self.user = None;
            }
            None => {}
        }
        o.lines
    }

    /// State of the open watch page, if it shows title `r`.
    fn page_session(&mut self, r: &str) -> Option<&mut Session> {
        self.page.as_mut().map(|p| &mut p.s).filter(|s| s.r == r)
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
        let Some(typed) = self.input.strip_prefix('/').or_else(|| self.input.strip_prefix('.')) else { return vec![] };
        if typed.contains(' ') {
            return vec![];
        }
        let typed = keys::command_word(&typed.to_lowercase());
        COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(&typed) || c.aliases.iter().any(|a| a.starts_with(&typed)))
            .collect()
    }

    /// The argument of a `/view` being typed, if that is what's in the prompt.
    fn view_arg(&self) -> Option<&str> {
        let body = self.input.strip_prefix('/').or_else(|| self.input.strip_prefix('.'))?;
        let (name, rest) = body.split_once(' ')?;
        let name = name.to_lowercase();
        let spec = find_command(&name).or_else(|| find_command(&keys::command_word(&name)))?;
        (spec.name == "view").then(|| rest.trim_start())
    }

    /// Suggestions for the current `/view` argument, if they are up to date.
    fn suggestions(&self) -> Option<&[Suggestion]> {
        let arg = self.view_arg()?;
        (!arg.is_empty()).then_some(self.suggest.items.as_slice())
    }

    /// Looks up what the typed `/view` argument points to, 250 ms after typing stops.
    fn suggest_tick(&mut self) {
        let arg = self.view_arg().unwrap_or("").trim().to_string();
        if arg != self.suggest.wanted {
            self.suggest.wanted = arg.clone();
            self.suggest.changed = Some(Instant::now());
        }
        if arg.is_empty() {
            self.suggest.items.clear();
            self.suggest.shown_for.clear();
            self.suggest.requested.clear();
            return;
        }
        if arg == self.suggest.requested || self.suggest.changed.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
            return;
        }
        self.suggest.requested = arg.clone();
        self.suggest.generation += 1;
        let generation = self.suggest.generation;
        let refs = self.anime_refs.clone();
        self.spawn_raw(move |c| {
            let res = (|| -> Result<Vec<Suggestion>> {
                // `#N`, an id, `mal:21` or a link point at one title; anything else is a search.
                let r = if let Some(n) = arg.strip_prefix('#') {
                    let n: usize = n.parse().context("після # має бути номер")?;
                    Some(refs.get(n.wrapping_sub(1)).cloned().with_context(|| format!("в останньому списку немає #{n}"))?)
                } else {
                    let r = crate::api::normalize_ref(&arg);
                    (r.contains(':') || r.parse::<i64>().is_ok()).then_some(r)
                };
                Ok(match r {
                    Some(r) => c.anime(&r).ok().as_ref().and_then(Suggestion::from_detail).into_iter().collect(),
                    None => c.search(&cli::search_query(&arg, 8, 1))?.items.iter().filter_map(Suggestion::from_short).collect(),
                })
            })();
            Msg::Suggested(generation, res)
        });
    }

    fn scroll_by(&mut self, delta: i64) {
        self.scroll = (self.scroll as i64 + delta).clamp(0, self.total_lines as i64) as usize;
    }

    fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        // Hotkeys work in any keyboard layout; typed text is left alone.
        let hk = keys::hotkey(k);
        if ctrl && matches!(hk.code, KeyCode::Char('p' | 'P')) {
            let shift = k.modifiers.contains(KeyModifiers::SHIFT) || hk.code == KeyCode::Char('P');
            self.open_palette(if shift { PMode::Actions } else { PMode::Search });
            return;
        }
        if ctrl && hk.code == KeyCode::Char('h') {
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
            self.page_key(hk);
            return;
        }
        if self.block_key(hk) {
            return;
        }
        // Picking what `/view` opens.
        if let Some(n) = self.suggestions().map(<[Suggestion]>::len).filter(|n| *n > 0) {
            let typed = self.view_arg().unwrap_or("").trim().to_string();
            let pick = self.suggest.items.get(self.ac_sel.min(n - 1)).map(|s| s.id.clone());
            match k.code {
                KeyCode::Up if !k.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.ac_sel = self.ac_sel.saturating_sub(1);
                    return;
                }
                KeyCode::Down if !k.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.ac_sel = (self.ac_sel + 1).min(n - 1);
                    return;
                }
                KeyCode::Tab => {
                    if let Some(id) = pick {
                        self.set_input(format!("/view {id}"));
                    }
                    return;
                }
                // A typed title opens the highlighted match; exact refs run as typed.
                KeyCode::Enter if self.suggest.shown_for == typed && !typed.starts_with('#') && crate::api::normalize_ref(&typed).parse::<i64>().is_err() && !typed.contains(':') => {
                    if let Some(id) = pick {
                        self.set_input(String::new());
                        self.submit(format!("/view {id}"));
                    }
                    return;
                }
                _ => {}
            }
        }
        let comps = self.completions();
        match if ctrl { hk.code } else { k.code } {
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
        if self.recent_cleared.is_some_and(|c| rec.at <= c) {
            return;
        }
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

    /// Opens the palette in `mode`; the same shortcut again closes it.
    fn open_palette(&mut self, mode: PMode) {
        if self.palette.as_ref().is_some_and(|p| p.mode == mode) {
            self.palette = None;
            return;
        }
        self.palette = Some(Palette {
            mode,
            query: String::new(),
            sel: 0,
            results: vec![],
            searched: String::new(),
            typed_at: Instant::now(),
            generation: 0,
        });
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
                    PItem { label: x.title.clone(), hint, action: PAction::Run(format!("/view {}", x.r)) }
                })
                .collect(),
            PMode::Actions => {
                let mut v = vec![
                    PItem { label: "Пошук аніме…".into(), hint: "Ctrl+P".into(), action: PAction::Search },
                    PItem { label: "Продовжити перегляд…".into(), hint: "Ctrl+H".into(), action: PAction::Recent },
                ];
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
                        run("Вийти", "/logout", "/logout"),
                    ]);
                } else {
                    v.push(run("Увійти", "/login", "/login"));
                }
                v.extend([
                    run("Довідка", "/help", "/help"),
                    run("Стерти історію", "/clear", "/clear"),
                    run("Стерти нещодавні", "/clear recent", "/clear recent"),
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
        };
        if p.mode == PMode::Search || p.query.is_empty() {
            return items;
        }
        let q = p.query.to_lowercase();
        items.into_iter().filter(|it| fuzzy(&q, &it.label.to_lowercase()) || fuzzy(&q, &it.hint.to_lowercase())).collect()
    }

    fn palette_key(&mut self, k: KeyEvent) {
        // The query is text; only Ctrl combinations are hotkeys here.
        let k = if k.modifiers.contains(KeyModifiers::CONTROL) { keys::hotkey(k) } else { k };
        let items_len = self.palette_items().len();
        let Some(p) = &mut self.palette else { return };
        match k.code {
            KeyCode::Esc => self.palette = None,
            // VS Code style: `>` in the search opens the menu, Backspace returns.
            KeyCode::Char('>') if p.mode == PMode::Search && p.query.is_empty() => {
                p.mode = PMode::Actions;
                p.sel = 0;
            }
            KeyCode::Backspace if p.mode == PMode::Actions && p.query.is_empty() => {
                p.mode = PMode::Search;
                p.sel = 0;
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => self.palette = None,
            KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                p.query.clear();
                p.sel = 0;
                p.typed_at = Instant::now();
            }
            KeyCode::Char(_) if k.modifiers.contains(KeyModifiers::CONTROL) => {}
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
        let bottom_h = 1 + 3 + 1; // gap, input with rules, status
        let transcript = Rect { height: area.height.saturating_sub(bottom_h), ..area };
        let y = transcript.y + transcript.height;
        let input_area = Rect { y: y + 1, height: 3, ..area };
        let status_area = Rect { y: y + 4, height: 1, ..area };

        self.draw_transcript(f, transcript);
        self.draw_input(f, input_area);
        self.draw_status(f, status_area);

        let comps = self.completions();
        if !comps.is_empty() && self.palette.is_none() {
            draw_completions(f, transcript, &comps, self.ac_sel.min(comps.len() - 1));
        } else if self.palette.is_none() {
            self.draw_suggestions(f, transcript);
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
        let mut action_before = false;
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
                if e.action {
                    // Page actions form a compact log: one line each, time on the right.
                    let prev_action = i > 0 && run[i].0 == 1 && start > 0 && action_before;
                    if !prev_action {
                        all.push(Line::raw(""));
                    }
                    all.push(action_line(e, body, width, spinner));
                } else {
                    all.push(Line::raw(""));
                    all.extend(body.iter().cloned());
                }
                action_before = e.action;
                owner.resize(all.len(), None);
                ranges.push((start, all.len()));
                continue;
            }
            action_before = false;
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
            vec![("Ctrl+P", "пошук"), ("Ctrl+⇧P", "меню"), ("Ctrl+H", "нещодавні"), ("Ctrl+↑", "блоки")]
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
        };
        let empty = match p.mode {
            PMode::Search if p.query.is_empty() => "почніть вводити назву",
            PMode::Recent if self.loading > 0 => "завантаження…",
            PMode::Recent => "ще нічого не дивились",
            PMode::Search if p.query != p.searched || self.loading > 0 => "шукаю…",
            _ => "нічого немає",
        };
        let placeholder = match p.mode {
            PMode::Search => "назва аніме",
            PMode::Actions => "> команда",
            PMode::Recent => "фільтр",
        };
        let mut footer = vec![("↑↓", "вибір"), ("⏎", "обрати")];
        if p.mode == PMode::Search {
            footer.push((">", "меню"));
        }
        footer.push(("Esc", "закрити"));
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

impl App {
    /// What `/view <arg>` would open, right above the prompt.
    fn draw_suggestions(&self, f: &mut Frame, above: Rect) {
        let Some(arg) = self.view_arg().map(str::trim) else { return };
        if arg.is_empty() {
            return;
        }
        let fresh = self.suggest.shown_for == arg;
        let items: Vec<ui::Item> = self
            .suggest
            .items
            .iter()
            .map(|s| {
                let mut it = ui::Item::new(s.title.clone(), s.meta.clone());
                it.pre = s.id.clone();
                it.dim = !fresh;
                it
            })
            .collect();
        let h = (items.len() as u16).clamp(1, 8);
        let w = 96.min(above.width.saturating_sub(4));
        let rect = Rect { x: above.x + 1, y: above.y + above.height.saturating_sub(h + 1), width: w, height: h + 1 };
        f.render_widget(Clear, rect);
        ui::fill(f, rect, ui::OVERLAY);
        let list = Rect { x: rect.x + 1, y: rect.y, width: rect.width - 2, height: h };
        if items.is_empty() {
            let msg = if fresh { "нічого не знайдено" } else { "шукаю…" };
            f.render_widget(Span::styled(msg, ui::muted()), Rect { x: list.x + 1, ..list });
        } else {
            ui::rows(f, list, &items, self.ac_sel.min(items.len() - 1), ui::OVERLAY, fresh);
        }
        let hints = ui::hints(&[("↑↓", "вибір"), ("Tab", "вставити id"), ("⏎", "відкрити")]);
        f.render_widget(Line::from(hints), Rect { x: rect.x + 2, y: rect.y + h, width: rect.width - 2, height: 1 });
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
    ui::rows(f, Rect { x: rect.x + 1, y: rect.y, width: rect.width - 2, height: h }, &items, sel, ui::OVERLAY, true);
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

fn action_line(e: &Entry, body: &[Line<'static>], width: u16, spinner: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    if e.pending {
        spans.push(Span::styled(spinner.to_string(), Style::new().fg(ACCENT)));
    } else if let Some(l) = body.first() {
        spans.extend(l.spans.iter().cloned());
    }
    let time = e.at.with_timezone(&Local).format("%H:%M").to_string();
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    spans.push(Span::raw(" ".repeat((width as usize).saturating_sub(used + time.len() + 2))));
    spans.push(Span::styled(time, Style::new().fg(FAINT)));
    Line::from(spans)
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
    anime_refs: Vec<String>,
    recent: Vec<Recent>,
    tx: Sender<Msg>,
    id: u64,
}

impl Snap {
    fn report(&self, lines: Lines) {
        let _ = self.tx.send(Msg::Append(self.id, lines));
    }

    fn need_key(&self) -> Result<()> {
        if !self.client.has_key() {
            bail!("потрібен вхід: /login");
        }
        Ok(())
    }

    /// `#N` from the last list, or any ref, link or title.
    fn anime(&self, arg: &str) -> Result<String> {
        if arg.is_empty() {
            bail!("яке аніме? /view <назва|#N>");
        }
        if let Some(n) = arg.strip_prefix('#') {
            let n: usize = n.parse().context("після # має бути номер")?;
            return self.anime_refs.get(n.wrapping_sub(1)).cloned().with_context(|| format!("в останньому списку немає #{n}"));
        }
        Ok(cli::resolve(&self.client, arg)?.0)
    }
}

/// Loads everything the watch page needs and opens it, picking up where the
/// title was left: the same team and the episode after the last one.
fn open_page(snap: &Snap, r: &str, detail: Option<AnimeDetail>) -> Result<Outcome> {
    let c = &snap.client;
    let d = match detail {
        Some(d) => d,
        None => c.anime(r)?,
    };
    let r = d.ids.mikai.map(|i| i.to_string()).unwrap_or_else(|| r.to_string());
    let player = c.player(&r)?;
    let entry = if c.has_key() { Some(cli::entry_of(c, &r)?) } else { None };
    let rec = snap.recent.iter().find(|x| x.r == r);
    let session = Session {
        r: r.clone(),
        title: d.titles.main().to_string(),
        url: d.url(),
        team: rec.and_then(|x| x.team.clone()),
        last_ep: rec.map(|x| x.ep),
        player: Some(player),
        entry,
    };
    let mut line = SLine::of("→ ", Tone::Accent).push(d.titles.main(), Tone::Bold);
    if let Some(x) = rec {
        line = line.push(format!("   продовжити після {} еп.", fmt_num(x.ep)), Tone::Dim);
    }
    Ok(Outcome { lines: vec![line], page: Some((d, session)), ..Default::default() })
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
                out.lines.push(dim("/view #N — відкрити"));
            }
            out.anime_refs = Some(page.items);
        }
        "view" => return open_page(snap, &snap.anime(args)?, None),
        "random" => {
            let d = c.random()?;
            let r = d.ids.mikai.map(|i| i.to_string()).unwrap_or_default();
            return open_page(snap, &r, Some(d));
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
