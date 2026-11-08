//! Output formatting shared by the CLI (ANSI) and the TUI (ratatui).
//!
//! Formatters produce [`SLine`]s: lines of styled segments that are plain data,
//! so the TUI can also save them to disk as part of the session history.

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{models::*, style};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tone {
    Plain,
    Bold,
    Dim,
    Accent,
    Green,
    Yellow,
    Red,
    Cyan,
    Magenta,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Seg(pub String, pub Tone);

/// One output line. `nowrap` lines (table rows) are cut instead of wrapped.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SLine {
    pub segs: Vec<Seg>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub nowrap: bool,
}

pub type Lines = Vec<SLine>;

impl SLine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn of(s: impl Into<String>, tone: Tone) -> Self {
        Self::new().push(s, tone)
    }

    pub fn push(mut self, s: impl Into<String>, tone: Tone) -> Self {
        let s = s.into();
        if !s.is_empty() {
            self.segs.push(Seg(s, tone));
        }
        self
    }

    /// Adds `s` padded or cut to `width` columns, followed by two spaces.
    pub fn col(self, s: &str, width: usize, tone: Tone) -> Self {
        self.push(style::pad(s, width), tone).push("  ", Tone::Plain)
    }

    pub fn rcol(self, s: &str, width: usize, tone: Tone) -> Self {
        let s = style::truncate(s, width);
        let fill = width.saturating_sub(unicode_width::UnicodeWidthStr::width(s.as_str()));
        self.push(" ".repeat(fill), Tone::Plain).push(s, tone).push("  ", Tone::Plain)
    }

    pub fn nowrap(mut self) -> Self {
        self.nowrap = true;
        self
    }

    pub fn text(&self) -> String {
        self.segs.iter().map(|s| s.0.as_str()).collect()
    }
}

pub fn plain(s: impl Into<String>) -> SLine {
    SLine::of(s, Tone::Plain)
}
pub fn dim(s: impl Into<String>) -> SLine {
    SLine::of(s, Tone::Dim)
}
pub fn err(s: impl Into<String>) -> SLine {
    SLine::of(s, Tone::Red)
}
pub fn blank() -> SLine {
    SLine::new()
}

pub fn to_ansi(line: &SLine) -> String {
    line.segs
        .iter()
        .map(|Seg(s, tone)| match tone {
            Tone::Plain => s.clone(),
            Tone::Bold => style::bold(s),
            Tone::Dim => style::dim(s),
            Tone::Accent => style::magenta(s),
            Tone::Green => style::green(s),
            Tone::Yellow => style::yellow(s),
            Tone::Red => style::red(s),
            Tone::Cyan => style::cyan(s),
            Tone::Magenta => style::magenta(s),
        })
        .collect()
}

pub fn print(lines: &[SLine]) {
    for l in lines {
        println!("{}", to_ansi(l).trim_end());
    }
}

// ---------- time ----------

pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

pub fn fmt_date(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%d.%m %H:%M").to_string()
}

fn fmt_date_str(s: Option<&str>) -> String {
    s.and_then(parse_time).map(fmt_date).unwrap_or_default()
}

// ---------- index column ----------

/// Leading column of list rows: `#3` when rows are numbered for later
/// reference, otherwise the Mikai id.
fn index_col(line: SLine, i: usize, numbered: bool, id: Option<i64>) -> SLine {
    if numbered {
        line.rcol(&format!("#{}", i + 1), 4, Tone::Dim)
    } else {
        line.rcol(&id.map(|i| i.to_string()).unwrap_or_default(), 6, Tone::Cyan)
    }
}

// ---------- anime ----------

pub fn anime_rows(items: &[AnimeShort], numbered: bool) -> Lines {
    if items.is_empty() {
        return vec![dim("Нічого не знайдено")];
    }
    items
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let meta = [
                a.format.as_deref().map(str::to_uppercase),
                a.year.map(|y| y.to_string()),
                short_episodes(&a.episodes).filter(|&e| e > 0).map(|e| format!("{e} еп.")),
                a.status.as_deref().map(|s| anime_status_label(s).to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            index_col(SLine::new(), i, numbered, a.ids.mikai)
                .push(a.titles.main(), Tone::Bold)
                .push("  ", Tone::Plain)
                .push(meta, Tone::Dim)
                .nowrap()
        })
        .collect()
}

fn season_label(s: &str) -> &str {
    match s {
        "winter" => "зима",
        "spring" => "весна",
        "summer" => "літо",
        "autumn" => "осінь",
        _ => "",
    }
}

/// `entry`: `None` — unknown (not logged in), `Some(None)` — not in lists.
pub fn anime_card(a: &AnimeDetail, entry: Option<Option<&ListEntry>>, numbered: bool) -> Lines {
    let mut out = vec![SLine::of(a.titles.main(), Tone::Bold)];
    if let Some(s) = a.titles.secondary() {
        out.push(dim(s));
    }
    let eps = &a.episodes;
    let eps_str = match (eps.total, eps.aired, eps.localized) {
        (Some(t), Some(ai), Some(l)) => {
            let t = if t == 0 { "?".into() } else { t.to_string() };
            Some(format!("{l} укр. / {ai} вийшло / {t} еп."))
        }
        _ => None,
    };
    let meta = [
        a.format.as_deref().map(str::to_uppercase),
        a.year.map(|y| format!("{} {y}", a.season.as_deref().map(season_label).unwrap_or("")).trim().to_string()),
        a.status.as_deref().map(|s| anime_status_label(s).to_string()),
        a.studio.clone(),
        a.age_rating.as_deref().map(str::to_uppercase),
        eps_str,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    out.push(plain(meta));

    let mut scores = SLine::new();
    for (name, val, prec) in [("Mikai", a.scores.mikai, 1), ("MAL", a.scores.mal, 2), ("Hikka", a.scores.hikka, 2)] {
        if let Some(s) = val {
            scores = scores.push(format!("{name} "), Tone::Plain).push(format!("{s:.prec$}"), Tone::Yellow).push("   ", Tone::Plain);
        }
    }
    if !scores.segs.is_empty() {
        out.push(scores);
    }
    if !a.genres.is_empty() {
        out.push(SLine::of(a.genres.iter().map(NamedUa::label).collect::<Vec<_>>().join(", "), Tone::Magenta));
    }
    match entry {
        Some(Some(e)) => out.push(entry_summary(e, &a.episodes)),
        Some(None) => out.push(dim("Немає в списках")),
        None => {}
    }
    if let Some(d) = &a.description {
        out.push(blank());
        out.extend(d.trim().lines().map(plain));
    }
    if a.licensed {
        out.push(blank());
        out.push(SLine::of("Ліцензовано в Україні — дивіться легально.", Tone::Yellow));
        if let Some(l) = &a.license {
            out.extend(license_lines(l));
        }
    } else if !a.releases.is_empty() {
        out.push(blank());
        out.push(SLine::of("Релізи", Tone::Bold));
        out.extend(releases(&a.releases, numbered));
    }
    if let Some(url) = a.url() {
        out.push(blank());
        out.push(dim(url));
    }
    out
}

fn license_lines(l: &Value) -> Lines {
    let mut out = vec![];
    let items = l.as_array().cloned().unwrap_or_else(|| vec![l.clone()]);
    for it in items {
        let name = ["name", "title", "platform", "type"].iter().find_map(|k| it.get(*k).and_then(Value::as_str));
        let url = it.get("url").and_then(Value::as_str);
        match (name, url) {
            (Some(n), Some(u)) => out.push(plain(format!("  {n}  ")).push(u, Tone::Cyan)),
            (None, Some(u)) => out.push(SLine::of(format!("  {u}"), Tone::Cyan)),
            _ => out.push(dim(format!("  {it}"))),
        }
    }
    out
}

pub fn entry_summary(e: &ListEntry, eps: &EpisodesInfo) -> SLine {
    let total = eps.total.filter(|&t| t > 0).map(|t| t.to_string()).unwrap_or_else(|| "?".into());
    SLine::new()
        .push("У списку: ", Tone::Plain)
        .push(list_status_label(&e.status), Tone::Bold)
        .push(format!(" · {}/{total}", e.episodes.map(fmt_num).unwrap_or_else(|| "0".into())), Tone::Green)
        .push(e.score.map(|s| format!(" · ★ {}", fmt_num(s))).unwrap_or_default(), Tone::Yellow)
        .push(if e.rewatching { " · повтор" } else { "" }, Tone::Dim)
}

pub fn releases(rels: &[Release], numbered: bool) -> Lines {
    rels.iter()
        .enumerate()
        .map(|(i, r)| {
            let line = if numbered { SLine::new().rcol(&format!("#{}", i + 1), 4, Tone::Dim) } else { SLine::new().push("  ", Tone::Plain) };
            line.col(&r.team_names(), 24, Tone::Bold)
                .col(r.kind_label(), 10, Tone::Dim)
                .col(&r.last_episode.map(|e| format!("до {} еп.", fmt_num(e))).unwrap_or_else(|| "–".into()), 11, Tone::Green)
                .push(fmt_date_str(r.updated_at.as_deref()), Tone::Dim)
                .push(if numbered { String::new() } else { format!("  {}", r.id) }, Tone::Dim)
                .nowrap()
        })
        .collect()
}

pub fn episodes(rel: &Release, watched: f64, provider: Option<&str>) -> Lines {
    let mut out = vec![
        SLine::new()
            .push(rel.team_names(), Tone::Bold)
            .push(format!("  {} · {}", rel.kind_label(), rel.id), Tone::Dim),
    ];
    for e in &rel.episodes {
        let mark = if e.number <= watched { "✓ " } else { "  " };
        let mut line = SLine::new().push(mark, Tone::Green).col(&format!("{} еп.", e.label()), 8, Tone::Bold);
        for (i, s) in e.sources.iter().enumerate() {
            let chosen = provider.map(|p| p.eq_ignore_ascii_case(&s.provider)).unwrap_or(i == 0);
            line = line.push(format!("{} ", s.provider), if chosen { Tone::Accent } else { Tone::Dim });
        }
        out.push(line.nowrap());
    }
    out
}

// ---------- feeds ----------

const WEEKDAYS: [(&str, &str); 7] = [
    ("monday", "Понеділок"),
    ("tuesday", "Вівторок"),
    ("wednesday", "Середа"),
    ("thursday", "Четвер"),
    ("friday", "Пʼятниця"),
    ("saturday", "Субота"),
    ("sunday", "Неділя"),
];

/// `today`, `mon`, `пн`… → `monday`.
pub fn weekday_key(input: &str) -> String {
    let d = input.to_lowercase();
    if d == "today" || d == "сьогодні" {
        return Local::now().format("%A").to_string().to_lowercase();
    }
    let ua = [("пн", 0), ("вт", 1), ("ср", 2), ("чт", 3), ("пт", 4), ("сб", 5), ("нд", 6)];
    if let Some((_, i)) = ua.iter().find(|(p, _)| d.starts_with(p)) {
        return WEEKDAYS[*i].0.to_string();
    }
    WEEKDAYS
        .iter()
        .find(|(k, l)| k.starts_with(&d) || l.to_lowercase().starts_with(&d))
        .map(|(k, _)| k.to_string())
        .unwrap_or(d)
}

/// Schedule rows; anime refs are returned in display order for `#N` references.
pub fn schedule(
    days: std::collections::BTreeMap<String, Vec<ScheduleItem>>,
    only: Option<&str>,
    numbered: bool,
) -> (Lines, Vec<AnimeShort>) {
    let mut days: Vec<_> = days.into_iter().collect();
    days.sort_by_key(|(k, _)| WEEKDAYS.iter().position(|(w, _)| w == k).unwrap_or(7));
    let mut out = vec![];
    let mut refs = vec![];
    for (key, mut items) in days {
        if only.is_some_and(|d| d != key) {
            continue;
        }
        items.sort_by_key(|i| i.airing);
        let label = WEEKDAYS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or(&key);
        out.push(SLine::of(label, Tone::Accent));
        for it in items {
            let time = it
                .airing
                .map(|t| t.with_timezone(&Local).format("%H:%M").to_string())
                .unwrap_or_else(|| "--:--".into());
            let line = if numbered { SLine::new().rcol(&format!("#{}", refs.len() + 1), 4, Tone::Dim) } else { SLine::new().push("  ", Tone::Plain) };
            out.push(
                line.col(&time, 5, Tone::Cyan)
                    .col(&it.episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default(), 8, Tone::Green)
                    .push(it.anime.titles.main(), Tone::Plain)
                    .nowrap(),
            );
            refs.push(it.anime);
        }
    }
    if out.is_empty() {
        out.push(dim("Нічого не виходить"));
    }
    (out, refs)
}

pub fn updates(items: &[Update], numbered: bool) -> Lines {
    items
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let team = u
                .team_name
                .clone()
                .unwrap_or_else(|| u.teams.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join(" × "));
            let line = if numbered { SLine::new().rcol(&format!("#{}", i + 1), 4, Tone::Dim) } else { SLine::new() };
            line.col(&u.published_at.map(fmt_date).unwrap_or_default(), 11, Tone::Dim)
                .col(&u.episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default(), 8, Tone::Green)
                .col(u.anime.titles.main(), 40, Tone::Bold)
                .push(format!("{team} · {} · {}", kind_label(&u.kind), u.provider.as_deref().unwrap_or("")), Tone::Dim)
                .nowrap()
        })
        .collect()
}

pub fn list_entries(items: &[ListEntry], numbered: bool) -> Lines {
    if items.is_empty() {
        return vec![dim("Порожньо")];
    }
    items.iter().enumerate().map(|(i, e)| list_entry_row(e, i, numbered)).collect()
}

pub fn list_entry_row(e: &ListEntry, i: usize, numbered: bool) -> SLine {
    let total = short_episodes(&e.anime.episodes).filter(|&t| t > 0).map(|t| t.to_string()).unwrap_or_else(|| "?".into());
    index_col(SLine::new(), i, numbered, e.anime.ids.mikai)
        .col(list_status_label(&e.status), 11, Tone::Dim)
        .col(&format!("{}/{total}", e.episodes.map(fmt_num).unwrap_or_else(|| "0".into())), 9, Tone::Green)
        .push(e.anime.titles.main(), Tone::Bold)
        .push(e.score.map(|s| format!("  ★ {}", fmt_num(s))).unwrap_or_default(), Tone::Yellow)
        .push(e.new_episodes.filter(|&n| n > 0).map(|n| format!("  +{n} нових")).unwrap_or_default(), Tone::Accent)
        .nowrap()
}

pub fn anime_of(v: &Value) -> AnimeShort {
    v.get("anime").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default()
}

pub fn history(items: &[Value], numbered: bool) -> Lines {
    if items.is_empty() {
        return vec![dim("Історія порожня")];
    }
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let anime = anime_of(item);
            let ep = item.get("episode").and_then(Value::as_f64).map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default();
            let when = ["watchedAt", "updatedAt", "createdAt"].iter().find_map(|k| item.get(*k).and_then(Value::as_str));
            index_col(SLine::new(), i, numbered, anime.ids.mikai)
                .col(&fmt_date_str(when), 11, Tone::Dim)
                .col(&ep, 8, Tone::Green)
                .push(anime.titles.main(), Tone::Bold)
                .nowrap()
        })
        .collect()
}

pub fn notifications(items: &[Value]) -> Lines {
    if items.is_empty() {
        return vec![dim("Сповіщень немає")];
    }
    items
        .iter()
        .map(|n| {
            let kind = n.get("type").and_then(Value::as_str).unwrap_or("?");
            let when = fmt_date_str(n.get("createdAt").and_then(Value::as_str));
            let text = match kind {
                "newEpisode" => {
                    let ne = n.get("newEpisode").cloned().unwrap_or_default();
                    let eps = ne
                        .get("episodes")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_f64).map(fmt_num).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    format!("{} — нові серії {eps}", anime_of(&ne).titles.main())
                }
                "system" => n.pointer("/system/text").and_then(Value::as_str).unwrap_or("").to_string(),
                other => other.to_string(),
            };
            SLine::new().col(&when, 11, Tone::Dim).push(text, Tone::Plain)
        })
        .collect()
}

pub fn me(me: &Me) -> Lines {
    let mut out = vec![];
    if me.authenticated {
        let user = me.user.as_ref().map(User::label).unwrap_or_else(|| "ключ без акаунта".into());
        out.push(SLine::of(user, Tone::Bold).push(format!(" · рівень {}", me.level.as_deref().unwrap_or("?")), Tone::Plain));
    } else {
        out.push(plain("Не увійшли"));
    }
    out.push(dim(format!(
        "ліміт {}/хв · сьогодні {} запитів · усього {}",
        me.rate_limit_per_min.unwrap_or(0),
        me.requests_today.unwrap_or(0),
        me.requests_total.unwrap_or(0)
    )));
    out
}

pub fn page_footer(page: Option<u64>, pages: Option<u64>, total: Option<u64>) -> Option<SLine> {
    match (page, pages) {
        (Some(p), Some(ps)) if ps > 1 => Some(dim(format!("сторінка {p}/{ps}, усього {}", total.unwrap_or(0)))),
        _ => None,
    }
}
