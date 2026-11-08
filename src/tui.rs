//! Interactive terminal UI (ratatui). Network calls run on short-lived worker
//! threads and report back over a channel, so the UI never blocks.

use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::Result;
use chrono::Local;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap},
};
use serde_json::json;

use crate::{
    api::{self, Client, Page},
    cli::{fmt_date, weekday_label, weekday_order},
    models::*,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Search,
    Schedule,
    Updates,
    List,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Search, "Пошук"),
    (Tab::Schedule, "Розклад"),
    (Tab::Updates, "Новинки"),
    (Tab::List, "Мій список"),
];

enum Msg {
    Search(u64, Result<Page<AnimeShort>>),
    Schedule(Result<Vec<(String, ScheduleItem)>>),
    Updates(Result<Page<Update>>),
    List(Result<Page<ListEntry>>),
    Detail(String, Result<AnimeDetail>),
    Player(String, Result<Player>),
    Entry(String, Result<Option<ListEntry>>),
    Saved(String, Result<ListEntry>),
}

struct AnimeView {
    r: String,
    detail: Option<AnimeDetail>,
    player: Option<Player>,
    entry: Option<ListEntry>,
    releases: TableState,
    scroll: u16,
}

impl AnimeView {
    fn releases(&self) -> &[Release] {
        match (&self.player, &self.detail) {
            (Some(p), _) => &p.releases,
            (None, Some(d)) => &d.releases,
            _ => &[],
        }
    }

    fn title(&self) -> String {
        self.detail
            .as_ref()
            .map(|d| d.titles.main().to_string())
            .or_else(|| self.player.as_ref().map(|p| p.anime.titles.main().to_string()))
            .unwrap_or_else(|| format!("#{}", self.r))
    }
}

struct EpisodesView {
    r: String,
    title: String,
    release: Release,
    entry: Option<ListEntry>,
    table: TableState,
    provider: Option<String>,
}

enum View {
    Anime(Box<AnimeView>),
    Episodes(Box<EpisodesView>),
}

struct App {
    client: Client,
    user: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,

    tab: Tab,
    loaded: [bool; 4],
    input: String,
    editing: bool,
    search_gen: u64,
    search: Vec<AnimeShort>,
    search_state: TableState,
    schedule: Vec<(String, ScheduleItem)>,
    schedule_state: TableState,
    updates: Vec<Update>,
    updates_state: TableState,
    list: Vec<ListEntry>,
    list_state: TableState,
    list_filter: Option<usize>,

    stack: Vec<View>,
    status: String,
    error: bool,
    loading: usize,
    help: bool,
    quit: bool,
}

pub fn run(client: Client, user: Option<String>) -> Result<()> {
    let mut terminal = ratatui::init();
    let result = App::new(client, user).run(&mut terminal);
    ratatui::restore();
    result
}

const ACCENT: Color = Color::Rgb(224, 69, 123);

impl App {
    fn new(client: Client, user: Option<String>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            client,
            user,
            tx,
            rx,
            tab: Tab::Search,
            loaded: [false; 4],
            input: String::new(),
            editing: false,
            search_gen: 0,
            search: vec![],
            search_state: TableState::default(),
            schedule: vec![],
            schedule_state: TableState::default(),
            updates: vec![],
            updates_state: TableState::default(),
            list: vec![],
            list_state: TableState::default(),
            list_filter: Some(0),
            stack: vec![],
            status: String::new(),
            error: false,
            loading: 0,
            help: false,
            quit: false,
        }
    }

    fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        self.load_tab();
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(80))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.on_key(k);
                    }
                }
            }
            while let Ok(msg) = self.rx.try_recv() {
                self.loading = self.loading.saturating_sub(1);
                self.on_msg(msg);
            }
        }
        Ok(())
    }

    fn spawn(&mut self, f: impl FnOnce(&Client) -> Msg + Send + 'static) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.loading += 1;
        thread::spawn(move || {
            let _ = tx.send(f(&client));
        });
    }

    fn info(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.error = false;
    }

    fn fail(&mut self, e: &anyhow::Error) {
        self.status = format!("{e:#}");
        self.error = true;
    }

    // ---------- loading ----------

    fn tab_index(&self) -> usize {
        TABS.iter().position(|(t, _)| *t == self.tab).unwrap()
    }

    fn load_tab(&mut self) {
        let i = self.tab_index();
        self.loaded[i] = true;
        match self.tab {
            Tab::Search => self.run_search(),
            Tab::Schedule => self.spawn(|c| {
                Msg::Schedule(c.schedule().map(|days| {
                    let mut days: Vec<_> = days.into_iter().collect();
                    days.sort_by_key(|(k, _)| weekday_order(k));
                    days.into_iter()
                        .flat_map(|(day, mut items)| {
                            items.sort_by_key(|i| i.airing);
                            items.into_iter().map(move |i| (day.clone(), i))
                        })
                        .collect()
                }))
            }),
            Tab::Updates => self.spawn(|c| Msg::Updates(c.updates(1, 100))),
            Tab::List => {
                if !self.client.has_key() {
                    self.info("Щоб бачити списки, увійдіть: mikai login");
                    return;
                }
                let status = self.list_filter.map(|i| LIST_STATUSES[i].to_string());
                self.spawn(move |c| Msg::List(c.my_list(status.as_deref(), 1, 100)));
            }
        }
    }

    fn run_search(&mut self) {
        self.search_gen += 1;
        let generation = self.search_gen;
        let text = self.input.trim().to_string();
        self.spawn(move |c| {
            let mut q: api::Query = vec![("limit", "100".into())];
            if text.is_empty() {
                q.push(("sort", "updated".into()));
            } else {
                q.push(("search", text));
            }
            Msg::Search(generation, c.search(&q))
        });
    }

    fn open_anime(&mut self, r: String) {
        self.stack.push(View::Anime(Box::new(AnimeView {
            r: r.clone(),
            detail: None,
            player: None,
            entry: None,
            releases: TableState::default(),
            scroll: 0,
        })));
        self.load_anime(r);
    }

    fn load_anime(&mut self, r: String) {
        let r2 = r.clone();
        self.spawn(move |c| Msg::Detail(r2.clone(), c.anime(&r2)));
        let r2 = r.clone();
        self.spawn(move |c| Msg::Player(r2.clone(), c.player(&r2)));
        if self.client.has_key() {
            self.spawn(move |c| {
                let res = match c.list_entry(&r) {
                    Ok(e) => Ok(Some(e)),
                    Err(e) if api::is_code(&e, "not_in_list") => Ok(None),
                    Err(e) => Err(e),
                };
                Msg::Entry(r, res)
            });
        }
    }

    fn save(&mut self, r: String, patch: serde_json::Value) {
        if !self.client.has_key() {
            self.info("Щоб змінювати списки, увійдіть: mikai login");
            return;
        }
        self.info("Зберігаю…");
        self.spawn(move |c| Msg::Saved(r.clone(), c.save_entry(&r, &patch)));
    }

    // ---------- messages ----------

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Search(generation, res) if generation == self.search_gen => match res {
                Ok(p) => {
                    self.search = p.items;
                    select_first(&mut self.search_state, self.search.len());
                    let total = p.total.unwrap_or(self.search.len() as u64);
                    self.info(if self.input.trim().is_empty() {
                        "Нещодавно оновлені".to_string()
                    } else {
                        format!("Знайдено: {total}")
                    });
                }
                Err(e) => self.fail(&e),
            },
            Msg::Search(..) => {}
            Msg::Schedule(res) => match res {
                Ok(items) => {
                    let today = Local::now().format("%A").to_string().to_lowercase();
                    self.schedule_state.select(Some(items.iter().position(|(d, _)| *d == today).unwrap_or(0)));
                    self.schedule = items;
                }
                Err(e) => self.fail(&e),
            },
            Msg::Updates(res) => match res {
                Ok(p) => {
                    self.updates = p.items;
                    select_first(&mut self.updates_state, self.updates.len());
                }
                Err(e) => self.fail(&e),
            },
            Msg::List(res) => match res {
                Ok(p) => {
                    self.list = p.items;
                    select_first(&mut self.list_state, self.list.len());
                }
                Err(e) => self.fail(&e),
            },
            Msg::Detail(r, res) => match res {
                Ok(d) => self.with_anime(&r, |v| v.detail = Some(d)),
                Err(e) => self.fail(&e),
            },
            Msg::Player(r, res) => match res {
                Ok(p) => self.with_anime(&r, |v| {
                    select_first(&mut v.releases, p.releases.len());
                    v.player = Some(p);
                }),
                Err(e) => self.fail(&e),
            },
            Msg::Entry(r, res) => match res {
                Ok(e) => self.with_anime(&r, |v| v.entry = e),
                Err(e) => self.fail(&e),
            },
            Msg::Saved(r, res) => match res {
                Ok(e) => {
                    self.info(format!(
                        "Збережено: {} · {} еп.",
                        list_status_label(&e.status),
                        e.episodes.map(fmt_num).unwrap_or_else(|| "0".into())
                    ));
                    for view in &mut self.stack {
                        match view {
                            View::Anime(v) if v.r == r => v.entry = Some(e.clone()),
                            View::Episodes(v) if v.r == r => v.entry = Some(e.clone()),
                            _ => {}
                        }
                    }
                    if let Some(item) = self.list.iter_mut().find(|i| i.anime.reference().as_deref() == Some(&r)) {
                        let anime = item.anime.clone();
                        *item = e;
                        item.anime = anime;
                    }
                }
                Err(e) => self.fail(&e),
            },
        }
    }

    fn with_anime(&mut self, r: &str, f: impl FnOnce(&mut AnimeView)) {
        if let Some(View::Anime(v)) = self.stack.iter_mut().rev().find(|v| matches!(v, View::Anime(a) if a.r == r)) {
            f(v);
        }
    }

    // ---------- keys ----------

    fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        if self.editing {
            match k.code {
                KeyCode::Enter => {
                    self.editing = false;
                    self.run_search();
                }
                KeyCode::Esc => self.editing = false,
                KeyCode::Backspace => {
                    self.input.pop();
                }
                KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => self.input.clear(),
                KeyCode::Char(c) => self.input.push(c),
                _ => {}
            }
            return;
        }

        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('/') => {
                self.stack.clear();
                self.tab = Tab::Search;
                self.editing = true;
            }
            KeyCode::Tab => self.switch_tab((self.tab_index() + 1) % TABS.len()),
            KeyCode::BackTab => self.switch_tab((self.tab_index() + TABS.len() - 1) % TABS.len()),
            KeyCode::Char(c @ '1'..='4') => self.switch_tab(c as usize - '1' as usize),
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                self.stack.pop();
            }
            _ => match self.stack.last() {
                Some(View::Anime(_)) => self.key_anime(k),
                Some(View::Episodes(_)) => self.key_episodes(k),
                None => self.key_tab(k),
            },
        }
    }

    fn switch_tab(&mut self, i: usize) {
        self.stack.clear();
        self.tab = TABS[i].0;
        if !self.loaded[i] {
            self.load_tab();
        }
    }

    fn key_tab(&mut self, k: KeyEvent) {
        let (state, len) = match self.tab {
            Tab::Search => (&mut self.search_state, self.search.len()),
            Tab::Schedule => (&mut self.schedule_state, self.schedule.len()),
            Tab::Updates => (&mut self.updates_state, self.updates.len()),
            Tab::List => (&mut self.list_state, self.list.len()),
        };
        if move_selection(state, len, k.code) {
            return;
        }
        let selected = state.selected();
        let anime = selected.and_then(|i| match self.tab {
            Tab::Search => self.search.get(i).cloned(),
            Tab::Schedule => self.schedule.get(i).map(|(_, s)| s.anime.clone()),
            Tab::Updates => self.updates.get(i).map(|u| u.anime.clone()),
            Tab::List => self.list.get(i).map(|e| e.anime.clone()),
        });
        match k.code {
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(r) = anime.and_then(|a| a.reference()) {
                    self.open_anime(r);
                }
            }
            KeyCode::Char('o') => {
                if let Some(url) = anime.and_then(|a| a.url()) {
                    self.open_url(&url);
                }
            }
            KeyCode::Char('r') => self.load_tab(),
            KeyCode::Char('i') | KeyCode::Char('s') if self.tab == Tab::Search => self.editing = true,
            KeyCode::Char('f') if self.tab == Tab::List => {
                self.list_filter = match self.list_filter {
                    None => Some(0),
                    Some(i) if i + 1 < LIST_STATUSES.len() => Some(i + 1),
                    Some(_) => None,
                };
                self.load_tab();
            }
            KeyCode::Char('+') | KeyCode::Char('=') if self.tab == Tab::List => {
                if let Some(e) = selected.and_then(|i| self.list.get(i)) {
                    if let Some(r) = e.anime.reference() {
                        let next = e.episodes.unwrap_or(0.0).floor() as i64 + 1;
                        self.save(r, json!({ "episodes": next }));
                    }
                }
            }
            _ => {}
        }
    }

    fn key_anime(&mut self, k: KeyEvent) {
        let Some(View::Anime(v)) = self.stack.last_mut() else { return };
        let len = v.releases().len();
        if move_selection(&mut v.releases, len, k.code) {
            return;
        }
        let r = v.r.clone();
        match k.code {
            KeyCode::Char('J') => v.scroll = v.scroll.saturating_add(1),
            KeyCode::Char('K') => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                let Some(rel) = v.releases.selected().and_then(|i| v.releases().get(i)).cloned() else { return };
                if rel.episodes.is_empty() {
                    self.info("Серії ще завантажуються…");
                    return;
                }
                let mut table = TableState::default();
                // Start at the next episode to watch.
                let watched = v.entry.as_ref().and_then(|e| e.episodes).unwrap_or(0.0);
                let next = rel.episodes.iter().position(|e| e.number > watched).unwrap_or(0);
                table.select(Some(next));
                let view = EpisodesView { r, title: v.title(), release: rel, entry: v.entry.clone(), table, provider: None };
                self.stack.push(View::Episodes(Box::new(view)));
            }
            KeyCode::Char('o') => {
                if let Some(url) = v.detail.as_ref().and_then(|d| d.url()) {
                    self.open_url(&url);
                }
            }
            KeyCode::Char('r') => {
                v.detail = None;
                v.player = None;
                self.load_anime(r);
            }
            KeyCode::Char('s') => {
                let cur = v.entry.as_ref().map(|e| e.status.clone());
                let next = match cur.and_then(|s| LIST_STATUSES.iter().position(|x| *x == s)) {
                    Some(i) => LIST_STATUSES[(i + 1) % LIST_STATUSES.len()],
                    None => "planned",
                };
                self.save(r, json!({ "status": next }));
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') => {
                let cur = v.entry.as_ref().and_then(|e| e.episodes).unwrap_or(0.0).floor() as i64;
                let next = if k.code == KeyCode::Char('-') { (cur - 1).max(0) } else { cur + 1 };
                let mut patch = json!({ "episodes": next });
                if v.entry.is_none() {
                    patch["status"] = json!("watching");
                }
                self.save(r, patch);
            }
            _ => {}
        }
    }

    fn key_episodes(&mut self, k: KeyEvent) {
        let Some(View::Episodes(v)) = self.stack.last_mut() else { return };
        let len = v.release.episodes.len();
        if move_selection(&mut v.table, len, k.code) {
            return;
        }
        let Some(ep) = v.table.selected().and_then(|i| v.release.episodes.get(i)).cloned() else { return };
        match k.code {
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                let src = v
                    .provider
                    .as_ref()
                    .and_then(|p| ep.sources.iter().find(|s| &s.provider == p))
                    .or_else(|| ep.sources.first());
                if let Some(src) = src.cloned() {
                    self.info(format!("Відкриваю {} еп. у {}", ep.label(), src.provider));
                    self.open_url(&src.embed_url);
                }
            }
            KeyCode::Char('p') => {
                // Cycle through the providers this release has.
                let mut all: Vec<String> = vec![];
                for e in &v.release.episodes {
                    for s in &e.sources {
                        if !all.contains(&s.provider) {
                            all.push(s.provider.clone());
                        }
                    }
                }
                if all.is_empty() {
                    return;
                }
                let i = v.provider.as_ref().and_then(|p| all.iter().position(|x| x == p));
                v.provider = Some(all[i.map(|i| (i + 1) % all.len()).unwrap_or(0)].clone());
                let p = v.provider.clone().unwrap();
                self.info(format!("Плеєр: {p}"));
            }
            KeyCode::Char('w') => {
                let r = v.r.clone();
                let n = ep.number.floor() as i64;
                let mut patch = json!({ "episodes": n });
                if v.entry.is_none() {
                    patch["status"] = json!("watching");
                }
                self.save(r, patch);
            }
            _ => {}
        }
    }

    fn open_url(&mut self, url: &str) {
        if let Err(e) = open::that_detached(url) {
            self.fail(&anyhow::Error::from(e).context("не вдалося відкрити браузер"));
        }
    }

    // ---------- drawing ----------

    fn draw(&mut self, f: &mut Frame) {
        let [top, main, bottom] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(f.area());

        let tabs = Tabs::new(TABS.iter().enumerate().map(|(i, (_, t))| format!("{} {t}", i + 1)))
            .select(self.tab_index())
            .highlight_style(Style::new().fg(ACCENT).bold())
            .divider("│");
        let [tabs_area, user_area] = Layout::horizontal([Constraint::Min(10), Constraint::Length(30)]).areas(top);
        f.render_widget(Line::from(vec![" mikai ".fg(ACCENT).bold()]), Rect { width: 7, ..tabs_area });
        f.render_widget(tabs, Rect { x: tabs_area.x + 7, width: tabs_area.width.saturating_sub(7), ..tabs_area });
        let user = self.user.clone().unwrap_or_else(|| "не увійшли".into());
        f.render_widget(Line::from(user).right_aligned().dim(), user_area);

        match self.stack.last_mut() {
            Some(View::Anime(v)) => draw_anime(f, main, v),
            Some(View::Episodes(v)) => draw_episodes(f, main, v),
            None => match self.tab {
                Tab::Search => self.draw_search(f, main),
                Tab::Schedule => self.draw_schedule(f, main),
                Tab::Updates => self.draw_updates(f, main),
                Tab::List => self.draw_list(f, main),
            },
        }

        let hints = match self.stack.last() {
            Some(View::Anime(_)) => "Enter серії · s статус · +/- прогрес · J/K опис · o сайт · Esc назад · ? довідка",
            Some(View::Episodes(_)) => "Enter дивитись · p плеєр · w переглянуто до цієї · Esc назад · ? довідка",
            None if self.editing => "Enter шукати · Esc скасувати",
            None => "Enter відкрити · / пошук · Tab вкладки · o сайт · r оновити · ? довідка · q вихід",
        };
        let spinner = if self.loading > 0 { "⟳ " } else { "" };
        let status = if self.status.is_empty() { Span::raw("") } else if self.error {
            Span::styled(format!("{} ", self.status), Style::new().fg(Color::Red))
        } else {
            Span::styled(format!("{} ", self.status), Style::new().fg(Color::Yellow))
        };
        f.render_widget(Line::from(vec![Span::raw(spinner).fg(ACCENT), status, Span::raw(hints).dim()]), bottom);

        if self.help {
            draw_help(f);
        }
    }

    fn draw_search(&mut self, f: &mut Frame, area: Rect) {
        let [input, list] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);
        let style = if self.editing { Style::new().fg(ACCENT) } else { Style::new().dim() };
        let text = if self.input.is_empty() && !self.editing {
            Line::from("натисніть / щоб шукати").dim()
        } else {
            Line::from(self.input.as_str())
        };
        f.render_widget(Paragraph::new(text).block(Block::bordered().title(" Пошук ").border_style(style)), input);
        if self.editing {
            let w = unicode_width::UnicodeWidthStr::width(self.input.as_str()) as u16;
            f.set_cursor_position((input.x + 1 + w, input.y + 1));
        }
        let rows = self.search.iter().map(|a| {
            Row::new(vec![
                Cell::from(a.titles.main().to_string()).bold(),
                Cell::from(a.format.as_deref().unwrap_or("").to_uppercase()).dim(),
                Cell::from(a.year.map(|y| y.to_string()).unwrap_or_default()),
                Cell::from(short_episodes(&a.episodes).filter(|&e| e > 0).map(|e| format!("{e} еп.")).unwrap_or_default()).green(),
                Cell::from(a.status.as_deref().map(anime_status_label).unwrap_or("").to_string()).dim(),
            ])
        });
        let table = Table::new(
            rows,
            [Constraint::Min(20), Constraint::Length(7), Constraint::Length(5), Constraint::Length(8), Constraint::Length(10)],
        )
        .header(header(["Назва", "Тип", "Рік", "Серії", "Статус"]));
        f.render_stateful_widget(styled_table(table, " Аніме "), list, &mut self.search_state);
    }

    fn draw_schedule(&mut self, f: &mut Frame, area: Rect) {
        let rows = self.schedule.iter().map(|(day, it)| {
            let time = it.airing.map(|t| t.with_timezone(&Local).format("%H:%M").to_string()).unwrap_or_else(|| "--:--".into());
            Row::new(vec![
                Cell::from(weekday_label(day).to_string()).fg(ACCENT),
                Cell::from(time).cyan(),
                Cell::from(it.episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default()).green(),
                Cell::from(it.anime.titles.main().to_string()).bold(),
            ])
        });
        let table = Table::new(rows, [Constraint::Length(10), Constraint::Length(6), Constraint::Length(8), Constraint::Min(20)])
            .header(header(["День", "Час", "Серія", "Назва"]));
        f.render_stateful_widget(styled_table(table, " Розклад "), area, &mut self.schedule_state);
    }

    fn draw_updates(&mut self, f: &mut Frame, area: Rect) {
        let rows = self.updates.iter().map(|u| {
            let team = u.team_name.clone().unwrap_or_else(|| u.teams.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join(" × "));
            Row::new(vec![
                Cell::from(u.published_at.map(fmt_date).unwrap_or_default()).dim(),
                Cell::from(u.episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default()).green(),
                Cell::from(u.anime.titles.main().to_string()).bold(),
                Cell::from(team).fg(ACCENT),
                Cell::from(format!("{} · {}", kind_label(&u.kind), u.provider.as_deref().unwrap_or(""))).dim(),
            ])
        });
        let table = Table::new(
            rows,
            [Constraint::Length(11), Constraint::Length(8), Constraint::Min(20), Constraint::Length(18), Constraint::Length(18)],
        )
        .header(header(["Коли", "Серія", "Назва", "Команда", ""]));
        f.render_stateful_widget(styled_table(table, " Новинки "), area, &mut self.updates_state);
    }

    fn draw_list(&mut self, f: &mut Frame, area: Rect) {
        let title = format!(
            " Мій список: {} (f — фільтр, + серія) ",
            self.list_filter.map(|i| list_status_label(LIST_STATUSES[i])).unwrap_or("усі")
        );
        let rows = self.list.iter().map(|e| {
            let total = short_episodes(&e.anime.episodes).filter(|&t| t > 0).map(|t| t.to_string()).unwrap_or_else(|| "?".into());
            Row::new(vec![
                Cell::from(e.anime.titles.main().to_string()).bold(),
                Cell::from(list_status_label(&e.status).to_string()).dim(),
                Cell::from(format!("{}/{total}", e.episodes.map(fmt_num).unwrap_or_else(|| "0".into()))).green(),
                Cell::from(e.score.map(|s| format!("★ {}", fmt_num(s))).unwrap_or_default()).yellow(),
                Cell::from(e.new_episodes.filter(|&n| n > 0).map(|n| format!("+{n} нових")).unwrap_or_default()).fg(ACCENT),
            ])
        });
        let table = Table::new(
            rows,
            [Constraint::Min(20), Constraint::Length(12), Constraint::Length(9), Constraint::Length(6), Constraint::Length(10)],
        )
        .header(header(["Назва", "Список", "Прогрес", "Оцінка", ""]));
        f.render_stateful_widget(styled_table(table, &title), area, &mut self.list_state);
    }
}

fn draw_anime(f: &mut Frame, area: Rect, v: &mut AnimeView) {
    let [info, rel] = Layout::vertical([Constraint::Percentage(55), Constraint::Min(5)]).areas(area);
    let mut lines: Vec<Line> = vec![];
    if let Some(d) = &v.detail {
        if let Some(s) = d.titles.secondary() {
            lines.push(Line::from(s.to_string()).dim());
        }
        let meta = [
            d.format.as_deref().map(str::to_uppercase),
            d.year.map(|y| y.to_string()),
            d.status.as_deref().map(|s| anime_status_label(s).to_string()),
            d.studio.clone(),
            d.age_rating.as_deref().map(str::to_uppercase),
            match (d.episodes.localized, d.episodes.aired, d.episodes.total) {
                (Some(l), Some(a), Some(t)) => Some(format!(
                    "{l} укр. / {a} вийшло / {} еп.",
                    if t == 0 { "?".into() } else { t.to_string() }
                )),
                _ => None,
            },
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        lines.push(Line::from(meta));
        let mut scores = vec![];
        for (name, val) in [("Mikai", d.scores.mikai), ("MAL", d.scores.mal), ("Hikka", d.scores.hikka)] {
            if let Some(s) = val {
                scores.push(Span::raw(format!("{name} ")));
                scores.push(Span::raw(format!("{s:.2}  ")).yellow().bold());
            }
        }
        lines.push(Line::from(scores));
        if !d.genres.is_empty() {
            lines.push(Line::from(d.genres.iter().map(NamedUa::label).collect::<Vec<_>>().join(", ")).fg(ACCENT));
        }
        let entry_line = match &v.entry {
            Some(e) => Line::from(vec![
                Span::raw("У списку: "),
                Span::raw(list_status_label(&e.status).to_string()).bold(),
                Span::raw(format!(" · переглянуто {}", e.episodes.map(fmt_num).unwrap_or_else(|| "0".into()))),
                Span::raw(e.score.map(|s| format!(" · ★ {}", fmt_num(s))).unwrap_or_default()).yellow(),
            ]),
            None => Line::from("Немає в списках (s — додати)").dim(),
        };
        lines.push(entry_line);
        lines.push(Line::from(""));
        if let Some(desc) = &d.description {
            for p in desc.lines() {
                lines.push(Line::from(p.to_string()));
            }
        }
        if d.licensed {
            lines.push(Line::from(""));
            lines.push(Line::from("Ліцензовано в Україні — дивіться легально (o — сторінка на mikai.me)").yellow());
        }
    } else {
        lines.push(Line::from("Завантаження…").dim());
    }
    let block = Block::bordered().title(Line::from(format!(" {} ", v.title())).fg(ACCENT).bold());
    f.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).scroll((v.scroll, 0)).block(block), info);

    let rows: Vec<Row> = v
        .releases()
        .iter()
        .map(|r| {
            Row::new(vec![
                Cell::from(r.team_names()).bold(),
                Cell::from(r.kind_label().to_string()).dim(),
                Cell::from(r.last_episode.map(|e| format!("до {} еп.", fmt_num(e))).unwrap_or_default()).green(),
                Cell::from(r.updated_at.as_deref().and_then(crate::cli::parse_time).map(fmt_date).unwrap_or_default()).dim(),
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Min(16), Constraint::Length(11), Constraint::Length(12), Constraint::Length(11)])
        .header(header(["Команда", "Трек", "Серії", "Оновлено"]));
    f.render_stateful_widget(styled_table(table, " Релізи "), rel, &mut v.releases);
}

fn draw_episodes(f: &mut Frame, area: Rect, v: &mut EpisodesView) {
    let watched = v.entry.as_ref().and_then(|e| e.episodes).unwrap_or(0.0);
    let provider = v.provider.clone();
    let rows: Vec<Row> = v
        .release
        .episodes
        .iter()
        .map(|e| {
            let mark = if e.number <= watched { "✓" } else { " " };
            let providers: Vec<Span> = e
                .sources
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let chosen = provider.as_ref().map(|p| p == &s.provider).unwrap_or(i == 0);
                    let span = Span::raw(format!("{} ", s.provider));
                    if chosen { span.fg(ACCENT).bold() } else { span.dim() }
                })
                .collect();
            Row::new(vec![
                Cell::from(mark).green(),
                Cell::from(format!("{} еп.", e.label())).bold(),
                Cell::from(e.added_at.as_deref().and_then(crate::cli::parse_time).map(fmt_date).unwrap_or_default()).dim(),
                Cell::from(Line::from(providers)),
            ])
        })
        .collect();
    let title = format!(" {} — {} · {} ", v.title, v.release.team_names(), v.release.kind_label());
    let table = Table::new(rows, [Constraint::Length(1), Constraint::Length(9), Constraint::Length(11), Constraint::Min(10)])
        .header(header(["", "Серія", "Додано", "Плеєри"]));
    f.render_stateful_widget(styled_table(table, &title), area, &mut v.table);
}

fn draw_help(f: &mut Frame) {
    let text = "\
Навігація
  ↑/↓ j/k         рух              g/G, PgUp/PgDn  початок/кінець, сторінка
  Enter → l       відкрити         Esc ← h         назад
  Tab, 1–4        вкладки          /               пошук
  o               сторінка на mikai.me у браузері
  r               оновити          q, Ctrl+C       вихід

Аніме
  s               змінити список (заплановано → дивлюсь → …)
  + / -           переглянуто серій
  J / K           гортати опис

Серії
  Enter           відкрити серію в браузері
  p               інший плеєр (ashdi, moon, fenix…)
  w               позначити переглянутим до цієї серії

Мій список
  f               фільтр за статусом      +   ще одна серія

Дані: mikai.me · відео належать провайдерам плеєрів";
    let area = f.area();
    let w = 84.min(area.width);
    let h = 24.min(area.height);
    let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(text).block(Block::bordered().title(" Довідка ").border_style(Style::new().fg(ACCENT))),
        rect,
    );
}

fn header<const N: usize>(cols: [&'static str; N]) -> Row<'static> {
    Row::new(cols).style(Style::new().add_modifier(Modifier::BOLD).fg(Color::Gray)).bottom_margin(0)
}

fn styled_table<'a>(t: Table<'a>, title: &str) -> Table<'a> {
    t.block(Block::bordered().title(title.to_string()))
        .row_highlight_style(Style::new().bg(Color::Rgb(60, 30, 45)))
        .highlight_symbol("▌")
        .column_spacing(2)
}

fn select_first(state: &mut TableState, len: usize) {
    state.select(if len == 0 { None } else { Some(0) });
}

/// Handles movement keys; returns `true` when the key was one of them.
fn move_selection(state: &mut TableState, len: usize, key: KeyCode) -> bool {
    if len == 0 {
        return matches!(key, KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k' | 'g' | 'G'));
    }
    let cur = state.selected().unwrap_or(0) as i64;
    let last = len as i64 - 1;
    let next = match key {
        KeyCode::Down | KeyCode::Char('j') => cur + 1,
        KeyCode::Up | KeyCode::Char('k') => cur - 1,
        KeyCode::PageDown => cur + 15,
        KeyCode::PageUp => cur - 15,
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => last,
        _ => return false,
    };
    state.select(Some(next.clamp(0, last) as usize));
    true
}
