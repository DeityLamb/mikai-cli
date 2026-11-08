//! Command-line interface: argument definitions and the command handlers.

use std::{thread, time::{Duration, Instant}};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, Utc};
use clap::{Args, Parser, Subcommand};
use reqwest::Method;
use serde_json::{Map, Value, json};

use crate::{
    api::{self, Client, normalize_ref},
    config::Config,
    models::*,
    style::*,
};

#[derive(Parser)]
#[command(
    name = "mikai",
    version,
    about = "Неофіційний клієнт для mikai.me: аніме з українським озвученням у терміналі",
    after_help = "Без команди відкривається інтерактивний TUI.\n\
                  Аніме можна вказати як 1272, mal:52991, al:154587, hikka:…, slug:… або посиланням на mikai.me."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Друкувати сиру JSON-відповідь API
    #[arg(long, global = true)]
    pub json: bool,

    /// Вимкнути кольори
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Ключ API замість збереженого
    #[arg(long, global = true, env = "MIKAI_API_KEY", hide_env_values = true)]
    pub key: Option<String>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Інтерактивний інтерфейс
    Tui,
    /// Пошук аніме в каталозі
    #[command(visible_alias = "s")]
    Search(SearchArgs),
    /// Картка аніме
    Show { anime: String },
    /// Випадкове аніме
    Random,
    /// Релізи (команди) аніме, а з --team — серії релізу
    #[command(visible_alias = "eps")]
    Episodes {
        anime: String,
        /// Команда: назва, slug, id або releaseId
        #[arg(short, long)]
        team: Option<String>,
        /// voice або sub
        #[arg(short, long)]
        kind: Option<String>,
    },
    /// Відкрити серію в браузері (без -e — сторінку аніме на mikai.me)
    Open {
        anime: String,
        /// Номер серії
        #[arg(short, long)]
        episode: Option<f64>,
        /// Команда: назва, slug, id або releaseId
        #[arg(short, long)]
        team: Option<String>,
        /// voice або sub
        #[arg(short, long)]
        kind: Option<String>,
        /// Бажаний плеєр (ashdi, moon, fenix…)
        #[arg(short, long)]
        provider: Option<String>,
        /// Лише надрукувати посилання
        #[arg(long)]
        print: bool,
    },
    /// Розклад виходу серій на тиждень
    Schedule {
        /// Лише один день: today, mon, tue… або monday…
        #[arg(short, long)]
        day: Option<String>,
    },
    /// Щойно додані серії
    Updates {
        #[arg(short, long, default_value_t = 20)]
        limit: u32,
        #[arg(short, long, default_value_t = 1)]
        page: u32,
    },
    /// Увійти в акаунт mikai.me
    Login {
        /// Готовий ключ mk_… з https://mikai.me/user/developers
        #[arg(long = "with-key", value_name = "KEY")]
        with_key: Option<String>,
        /// Рівень доступу для входу за посиланням
        #[arg(long, default_value = "write", value_parser = ["read", "write"])]
        level: String,
        /// clientId зареєстрованого застосунку
        #[arg(long, env = "MIKAI_CLIENT_ID")]
        client_id: Option<String>,
        /// Не відкривати браузер
        #[arg(long)]
        no_browser: bool,
    },
    /// Вийти й видалити ключ
    Logout {
        /// Лише забути ключ локально, не видаляючи його на сервері
        #[arg(long)]
        local: bool,
    },
    /// Акаунт, рівень ключа та ліміти
    Whoami,
    /// Мої списки
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Історія переглядів
    History {
        #[arg(short, long, default_value_t = 20)]
        limit: u32,
    },
    /// Сповіщення
    #[command(visible_alias = "notif")]
    Notifications {
        /// unread, read або all
        #[arg(short, long, default_value = "unread")]
        filter: String,
        /// Позначити всі прочитаними
        #[arg(long)]
        read_all: bool,
    },
    /// Українська транслітерація
    Translit {
        text: Vec<String>,
        /// ja, zh, ko або en
        #[arg(short, long)]
        lang: Option<String>,
    },
    /// Довільний запит до API, друкує JSON
    Api {
        /// Шлях після /public/v1, напр. genres або anime/1272/staff
        path: String,
        #[arg(short = 'X', long, default_value = "GET")]
        method: String,
        /// Параметри запиту key=value
        #[arg(short, long = "query", value_name = "K=V")]
        query: Vec<String>,
        /// JSON-тіло
        #[arg(short, long)]
        data: Option<String>,
    },
}

#[derive(Args)]
pub struct SearchArgs {
    /// Назва українською, англійською чи оригінальною
    pub query: Vec<String>,
    /// Жанри через кому (name з /genres)
    #[arg(short, long)]
    pub genres: Option<String>,
    /// finished, ongoing, announce…
    #[arg(short, long)]
    pub status: Option<String>,
    /// tv, movie, ova, ona…
    #[arg(short, long)]
    pub format: Option<String>,
    /// Рік або роки через кому
    #[arg(short, long)]
    pub year: Option<String>,
    /// name, year, updated, views, mal_rating
    #[arg(long)]
    pub sort: Option<String>,
    /// asc або desc
    #[arg(long)]
    pub order: Option<String>,
    #[arg(short, long, default_value_t = 20)]
    pub limit: u32,
    #[arg(short, long, default_value_t = 1)]
    pub page: u32,
}

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct ListArgs {
    #[command(subcommand)]
    pub cmd: Option<ListCmd>,
    /// watching, planned, completed, paused, dropped
    #[arg(short, long)]
    pub status: Option<String>,
    #[arg(short, long, default_value_t = 50)]
    pub limit: u32,
    #[arg(short, long, default_value_t = 1)]
    pub page: u32,
}

#[derive(Subcommand)]
pub enum ListCmd {
    /// Запис одного аніме
    Get { anime: String },
    /// Додати або змінити запис
    Set {
        anime: String,
        /// watching, planned, completed, paused, dropped
        #[arg(short, long)]
        status: Option<String>,
        /// Переглянуто до цієї серії
        #[arg(short, long)]
        episodes: Option<i64>,
        /// Оцінка 0.5–10, 0 — прибрати
        #[arg(long)]
        score: Option<f64>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Позначити ще одну серію переглянутою
    Inc { anime: String },
    /// Прибрати з усіх списків
    Rm { anime: String },
    /// Почати повторний перегляд (--cancel — скасувати)
    Rewatch {
        anime: String,
        #[arg(long)]
        cancel: bool,
    },
}

pub struct Ctx {
    pub client: Client,
    pub config: Config,
    pub json: bool,
}

pub fn run(cmd: Command, ctx: &mut Ctx) -> Result<()> {
    match cmd {
        Command::Tui => unreachable!("handled in main"),
        Command::Search(a) => search(ctx, a),
        Command::Show { anime } => show(ctx, &resolve(ctx, &anime)?),
        Command::Random => {
            if ctx.json {
                return print_raw(ctx, "anime/random", &[]);
            }
            let a = ctx.client.random()?;
            print_detail(&a);
            Ok(())
        }
        Command::Episodes { anime, team, kind } => episodes(ctx, &resolve(ctx, &anime)?, team, kind),
        Command::Open { anime, episode, team, kind, provider, print } => {
            open_episode(ctx, &resolve(ctx, &anime)?, episode, team, kind, provider, print)
        }
        Command::Schedule { day } => schedule(ctx, day),
        Command::Updates { limit, page } => updates(ctx, limit, page),
        Command::Login { with_key, level, client_id, no_browser } => login(ctx, with_key, &level, client_id, no_browser),
        Command::Logout { local } => logout(ctx, local),
        Command::Whoami => whoami(ctx),
        Command::List(a) => list(ctx, a),
        Command::History { limit } => history(ctx, limit),
        Command::Notifications { filter, read_all } => notifications(ctx, &filter, read_all),
        Command::Translit { text, lang } => {
            let r = ctx.client.translit(&text.join(" "), lang.as_deref())?;
            match r.get("text").and_then(Value::as_str) {
                Some(t) if !ctx.json => println!("{t}"),
                _ => println!("{}", serde_json::to_string_pretty(&r)?),
            }
            Ok(())
        }
        Command::Api { path, method, query, data } => {
            let method: Method = method.to_uppercase().parse().context("невідомий HTTP-метод")?;
            let q: Vec<(&str, String)> = query
                .iter()
                .map(|kv| kv.split_once('=').map(|(k, v)| (k, v.to_string())).context("параметр має вигляд key=value"))
                .collect::<Result<_>>()?;
            let body = data.map(|d| serde_json::from_str::<Value>(&d)).transpose().context("тіло -d не є JSON")?;
            let v = ctx.client.raw(method, &path, &q, body.as_ref())?;
            println!("{}", serde_json::to_string_pretty(&v)?);
            Ok(())
        }
    }
}

/// Turns user input into an API `{ref}`. Anything that is not an id, a prefixed
/// ref or a link is treated as a title and resolved to the best search hit.
fn resolve(ctx: &Ctx, input: &str) -> Result<String> {
    let r = normalize_ref(input);
    if r.contains(':') || r.parse::<i64>().is_ok() {
        return Ok(r);
    }
    let page = ctx.client.search(&[("search", r.clone()), ("limit", "1".into())])?;
    let hit = page.items.first().with_context(|| format!("аніме «{r}» не знайдено"))?;
    eprintln!("{}", dim(&format!("→ {} (#{})", hit.titles.main(), hit.ids.mikai.unwrap_or(0))));
    hit.reference().context("у результату немає id")
}

fn print_raw(ctx: &Ctx, path: &str, q: &[(&str, String)]) -> Result<()> {
    let v = ctx.client.raw(Method::GET, path, q, None)?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

// ---------- catalogue ----------

fn search(ctx: &Ctx, a: SearchArgs) -> Result<()> {
    let mut q: Vec<(&'static str, String)> = vec![("limit", a.limit.to_string()), ("page", a.page.to_string())];
    let text = a.query.join(" ");
    if !text.is_empty() {
        q.push(("search", text));
    }
    for (k, v) in [
        ("genres", a.genres),
        ("statuses", a.status),
        ("formats", a.format),
        ("years", a.year),
        ("sort", a.sort),
        ("order", a.order),
    ] {
        if let Some(v) = v {
            q.push((k, v));
        }
    }
    if ctx.json {
        return print_raw(ctx, "anime", &q);
    }
    let page = ctx.client.search(&q)?;
    if page.items.is_empty() {
        println!("{}", dim("Нічого не знайдено"));
        return Ok(());
    }
    for a in &page.items {
        print_short_row(a);
    }
    print_page_footer(page.page, page.pages, page.total);
    Ok(())
}

fn print_short_row(a: &AnimeShort) {
    let id = a.ids.mikai.map(|i| i.to_string()).unwrap_or_default();
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
    println!(
        "{}  {}  {}",
        cyan(&format!("{id:>6}")),
        bold(a.titles.main()),
        dim(&meta)
    );
}

fn print_page_footer(page: Option<u64>, pages: Option<u64>, total: Option<u64>) {
    if let (Some(p), Some(ps)) = (page, pages) {
        if ps > 1 {
            println!("{}", dim(&format!("сторінка {p}/{ps}, усього {} (--page N)", total.unwrap_or(0))));
        }
    }
}

fn show(ctx: &Ctx, r: &str) -> Result<()> {
    if ctx.json {
        return print_raw(ctx, &format!("anime/{r}"), &[]);
    }
    let a = ctx.client.anime(r)?;
    print_detail(&a);
    Ok(())
}

fn print_detail(a: &AnimeDetail) {
    println!("{}", bold(a.titles.main()));
    if let Some(s) = a.titles.secondary() {
        println!("{}", dim(s));
    }
    let eps = &a.episodes;
    let eps_str = match (eps.total, eps.aired, eps.localized) {
        (Some(t), Some(ai), Some(l)) => {
            let t = if t == 0 { "?".into() } else { t.to_string() };
            format!("{l} укр. / {ai} вийшло / {t} еп.")
        }
        _ => String::new(),
    };
    let meta = [
        a.format.as_deref().map(str::to_uppercase),
        a.year.map(|y| format!("{} {y}", a.season.as_deref().map(season_label).unwrap_or(""))),
        a.status.as_deref().map(|s| anime_status_label(s).to_string()),
        a.studio.clone(),
        a.age_rating.as_deref().map(str::to_uppercase),
        Some(eps_str).filter(|s| !s.is_empty()),
    ]
    .into_iter()
    .flatten()
    .map(|s| s.trim().to_string())
    .collect::<Vec<_>>()
    .join(" · ");
    println!("{meta}");
    let sc = &a.scores;
    let mut scores = vec![];
    if let Some(s) = sc.mikai {
        scores.push(format!("Mikai {}", yellow(&format!("{s:.1}"))));
    }
    if let Some(s) = sc.mal {
        scores.push(format!("MAL {}", yellow(&format!("{s:.2}"))));
    }
    if let Some(s) = sc.hikka {
        scores.push(format!("Hikka {}", yellow(&format!("{s:.2}"))));
    }
    if !scores.is_empty() {
        println!("{}", scores.join("  "));
    }
    if !a.genres.is_empty() {
        println!("{}", magenta(&a.genres.iter().map(NamedUa::label).collect::<Vec<_>>().join(", ")));
    }
    if let Some(d) = &a.description {
        println!("\n{}", d.trim());
    }
    if a.licensed {
        println!("\n{}", yellow("Ліцензовано в Україні — дивіться легально:"));
        if let Some(l) = &a.license {
            println!("{}", serde_json::to_string_pretty(l).unwrap_or_default());
        }
    } else if !a.releases.is_empty() {
        println!("\n{}", bold("Релізи"));
        print_releases(&a.releases);
    }
    if let Some(url) = a.url() {
        println!("\n{}", dim(&url));
    }
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

fn print_releases(releases: &[Release]) {
    for r in releases {
        let eps = r.last_episode.map(fmt_num).unwrap_or_else(|| "–".into());
        let upd = r.updated_at.as_deref().and_then(parse_time).map(fmt_date).unwrap_or_default();
        println!(
            "  {} {}  {}  {}",
            pad(&r.team_names(), 28),
            pad(r.kind_label(), 10),
            green(&pad(&format!("до {eps} еп."), 12)),
            dim(&format!("{upd}  {}", r.id))
        );
    }
}

fn episodes(ctx: &Ctx, r: &str, team: Option<String>, kind: Option<String>) -> Result<()> {
    if ctx.json {
        return print_raw(ctx, &format!("anime/{r}/player"), &[]);
    }
    let p = ctx.client.player(r)?;
    println!("{}", bold(p.anime.titles.main()));
    if p.licensed {
        println!("{}", yellow("Ліцензовано в Україні, джерел немає."));
        return Ok(());
    }
    let releases: Vec<&Release> = p
        .releases
        .iter()
        .filter(|rel| kind.as_deref().is_none_or(|k| rel.kind == k))
        .filter(|rel| team.as_deref().is_none_or(|t| rel.matches(t)))
        .collect();
    if releases.is_empty() {
        bail!("немає релізів за цим фільтром");
    }
    if team.is_none() {
        print_releases(&releases.into_iter().cloned().collect::<Vec<_>>());
        println!("{}", dim("Серії релізу: mikai episodes <аніме> --team <команда>"));
        return Ok(());
    }
    for rel in releases {
        println!("\n{} {}", bold(&rel.team_names()), dim(&format!("({}, {})", rel.kind_label(), rel.id)));
        for ep in &rel.episodes {
            let providers = ep.sources.iter().map(|s| s.provider.as_str()).collect::<Vec<_>>().join(", ");
            println!("  {}  {}", green(&pad(&format!("{} еп.", ep.label()), 9)), dim(&providers));
        }
    }
    Ok(())
}

fn open_episode(
    ctx: &Ctx,
    r: &str,
    episode: Option<f64>,
    team: Option<String>,
    kind: Option<String>,
    provider: Option<String>,
    print: bool,
) -> Result<()> {
    let url = match episode {
        None => {
            let a = ctx.client.anime(r)?;
            a.url().context("у аніме немає сторінки")?
        }
        Some(n) => {
            let p = ctx.client.player(r)?;
            if p.licensed {
                bail!("аніме ліцензоване в Україні, джерел немає");
            }
            let (rel, src) = pick_source(&p.releases, n, team.as_deref(), kind.as_deref(), provider.as_deref())
                .with_context(|| format!("серію {} не знайдено за цим фільтром", fmt_num(n)))?;
            if !print {
                eprintln!("{} · {} · {}", rel.team_names(), rel.kind_label(), src.provider);
            }
            src.embed_url.clone()
        }
    };
    if print {
        println!("{url}");
    } else {
        eprintln!("{}", dim(&url));
        open::that_detached(&url).context("не вдалося відкрити браузер")?;
    }
    Ok(())
}

/// First release (in API order) that matches the filters and has episode `n`,
/// and its source from the preferred provider (or the first one).
pub fn pick_source<'a>(
    releases: &'a [Release],
    n: f64,
    team: Option<&str>,
    kind: Option<&str>,
    provider: Option<&str>,
) -> Option<(&'a Release, &'a Source)> {
    releases
        .iter()
        .filter(|r| kind.is_none_or(|k| r.kind == k))
        .filter(|r| team.is_none_or(|t| r.matches(t)))
        .find_map(|r| {
            let ep = r.episodes.iter().find(|e| e.number == n)?;
            let src = provider
                .and_then(|p| ep.sources.iter().find(|s| s.provider.eq_ignore_ascii_case(p)))
                .or_else(|| ep.sources.first())?;
            Some((r, src))
        })
}

const WEEKDAYS: [(&str, &str); 7] = [
    ("monday", "Понеділок"),
    ("tuesday", "Вівторок"),
    ("wednesday", "Середа"),
    ("thursday", "Четвер"),
    ("friday", "Пʼятниця"),
    ("saturday", "Субота"),
    ("sunday", "Неділя"),
];

pub fn weekday_label(key: &str) -> &str {
    WEEKDAYS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or(key)
}

pub fn weekday_order(key: &str) -> usize {
    WEEKDAYS.iter().position(|(k, _)| *k == key).unwrap_or(7)
}

fn schedule(ctx: &Ctx, day: Option<String>) -> Result<()> {
    if ctx.json {
        return print_raw(ctx, "schedule", &[]);
    }
    let day = day.map(|d| {
        let d = d.to_lowercase();
        if d == "today" || d == "сьогодні" {
            Local::now().format("%A").to_string().to_lowercase()
        } else {
            WEEKDAYS.iter().find(|(k, _)| k.starts_with(&d)).map(|(k, _)| k.to_string()).unwrap_or(d)
        }
    });
    let sched = ctx.client.schedule()?;
    let mut days: Vec<_> = sched.into_iter().collect();
    days.sort_by_key(|(k, _)| weekday_order(k));
    for (key, mut items) in days {
        if day.as_deref().is_some_and(|d| d != key) {
            continue;
        }
        items.sort_by_key(|i| i.airing);
        println!("{}", bold(weekday_label(&key)));
        for it in items {
            let time = it.airing.map(|t| t.with_timezone(&Local).format("%H:%M").to_string()).unwrap_or_else(|| "--:--".into());
            let ep = it.episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default();
            println!(
                "  {}  {}  {}",
                cyan(&time),
                green(&pad(&ep, 8)),
                it.anime.titles.main()
            );
        }
    }
    Ok(())
}

fn updates(ctx: &Ctx, limit: u32, page: u32) -> Result<()> {
    if ctx.json {
        return print_raw(ctx, "updates", &[("limit", limit.to_string()), ("page", page.to_string())]);
    }
    let p = ctx.client.updates(page, limit)?;
    for u in &p.items {
        let when = u.published_at.map(fmt_date).unwrap_or_default();
        let team = u.team_name.clone().unwrap_or_else(|| u.teams.iter().map(|t| t.name.clone()).collect::<Vec<_>>().join(" × "));
        println!(
            "{}  {}  {}  {}",
            dim(&pad(&when, 11)),
            green(&pad(&u.episode.map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default(), 8)),
            bold(&pad(u.anime.titles.main(), 44)),
            dim(&format!("{team} · {} · {}", kind_label(&u.kind), u.provider.as_deref().unwrap_or("")))
        );
    }
    Ok(())
}

// ---------- account ----------

fn require_key(ctx: &Ctx) -> Result<()> {
    if !ctx.client.has_key() {
        bail!("потрібен вхід: mikai login");
    }
    Ok(())
}

fn login(ctx: &mut Ctx, with_key: Option<String>, level: &str, client_id: Option<String>, no_browser: bool) -> Result<()> {
    if let Some(key) = with_key {
        ctx.client.set_key(Some(key.clone()));
        let me = ctx.client.me().context("ключ не підійшов")?;
        if !me.authenticated {
            bail!("ключ не підійшов");
        }
        ctx.config.key = Some(key);
        ctx.config.level = me.level.clone();
        ctx.config.user = me.user.as_ref().map(User::label);
        ctx.config.save()?;
        println!(
            "Увійшли як {} (рівень {})",
            bold(ctx.config.user.as_deref().unwrap_or("?")),
            me.level.as_deref().unwrap_or("?")
        );
        return Ok(());
    }

    let client_id = client_id.or_else(|| ctx.config.client_id()).context(
        "немає clientId застосунку.\n\
         Увійдіть готовим ключем: створіть його на https://mikai.me/user/developers і виконайте\n  \
         mikai login --with-key mk_…\n\
         або передайте clientId: mikai login --client-id mkc_…",
    )?;

    let start = ctx.client.device_start(&client_id, level)?;
    println!("Підтвердіть вхід у браузері:\n\n  {}\n", bold(&start.verification_url));
    if !no_browser {
        let _ = open::that_detached(&start.verification_url);
    }
    eprintln!("{}", dim("Чекаю на підтвердження… (Ctrl+C — скасувати)"));

    let deadline = Instant::now() + Duration::from_secs(start.expires_in.max(60));
    let mut interval = start.interval.max(1);
    let token = loop {
        if Instant::now() > deadline {
            bail!("посилання протухло, спробуйте ще раз");
        }
        thread::sleep(Duration::from_secs(interval));
        match ctx.client.device_token(&client_id, &start.device_code) {
            Ok(t) => break t,
            Err(e) if api::is_code(&e, "authorization_pending") => {}
            Err(e) if api::is_code(&e, "slow_down") => interval += 5,
            Err(e) if api::is_code(&e, "access_denied") => bail!("вхід відхилено"),
            Err(e) if api::is_code(&e, "expired_token") => bail!("посилання протухло, спробуйте ще раз"),
            Err(e) => {
                let _ = ctx.client.device_cancel(&client_id, &start.device_code);
                return Err(e);
            }
        }
    };

    ctx.config.key = Some(token.key);
    ctx.config.level = token.level.clone();
    ctx.config.user = token.user.as_ref().map(User::label);
    ctx.config.client_id = Some(client_id);
    ctx.config.save()?;
    println!(
        "{} Увійшли як {} (рівень {})",
        green("✓"),
        bold(ctx.config.user.as_deref().unwrap_or("?")),
        token.level.as_deref().unwrap_or("?")
    );
    if token.level.as_deref() == Some("read") && level == "write" {
        println!("{}", yellow("Дозволено лише читання: змінювати списки не вийде."));
    }
    Ok(())
}

fn logout(ctx: &mut Ctx, local: bool) -> Result<()> {
    if ctx.config.key.is_none() {
        println!("Ви й так не увійшли.");
        return Ok(());
    }
    if !local {
        if let Err(e) = ctx.client.delete_key() {
            eprintln!("{} {e:#}", yellow("Не вдалося видалити ключ на сервері:"));
        }
    }
    ctx.config.key = None;
    ctx.config.level = None;
    ctx.config.user = None;
    ctx.config.save()?;
    println!("Вийшли.");
    Ok(())
}

fn whoami(ctx: &Ctx) -> Result<()> {
    if ctx.json {
        return print_raw(ctx, "me", &[]);
    }
    let me = ctx.client.me()?;
    if me.authenticated {
        let user = me.user.as_ref().map(User::label).unwrap_or_else(|| "ключ без акаунта".into());
        println!("{} · рівень {}", bold(&user), me.level.as_deref().unwrap_or("?"));
    } else {
        println!("Не увійшли (mikai login)");
    }
    println!(
        "{}",
        dim(&format!(
            "ліміт {}/хв · сьогодні {} запитів · усього {}",
            me.rate_limit_per_min.unwrap_or(0),
            me.requests_today.unwrap_or(0),
            me.requests_total.unwrap_or(0)
        ))
    );
    Ok(())
}

fn list(ctx: &Ctx, a: ListArgs) -> Result<()> {
    require_key(ctx)?;
    match a.cmd {
        None => {
            let mut q = vec![("page", a.page.to_string()), ("limit", a.limit.to_string())];
            if let Some(s) = &a.status {
                q.push(("status", s.clone()));
            }
            if ctx.json {
                return print_raw(ctx, "me/list", &q);
            }
            let p = ctx.client.my_list(a.status.as_deref(), a.page, a.limit)?;
            if p.items.is_empty() {
                println!("{}", dim("Порожньо"));
            }
            for e in &p.items {
                print_entry_row(e);
            }
            print_page_footer(p.page, p.pages, p.total);
        }
        Some(ListCmd::Get { anime }) => {
            let r = resolve(ctx, &anime)?;
            if ctx.json {
                return print_raw(ctx, &format!("me/list/{r}"), &[]);
            }
            match ctx.client.list_entry(&r) {
                Ok(e) => print_entry_row(&e),
                Err(e) if api::is_code(&e, "not_in_list") => println!("{}", dim("Немає в списках")),
                Err(e) => return Err(e),
            }
        }
        Some(ListCmd::Set { anime, status, episodes, score, note }) => {
            let mut patch = Map::new();
            if let Some(s) = status {
                patch.insert("status".into(), json!(s));
            }
            if let Some(e) = episodes {
                patch.insert("episodes".into(), json!(e));
            }
            if let Some(s) = score {
                patch.insert("score".into(), if s == 0.0 { Value::Null } else { json!(s) });
            }
            if let Some(n) = note {
                patch.insert("note".into(), json!(n));
            }
            if patch.is_empty() {
                bail!("нічого змінювати: вкажіть --status, --episodes, --score чи --note");
            }
            let e = ctx.client.save_entry(&resolve(ctx, &anime)?, &Value::Object(patch))?;
            print_entry_row(&e);
        }
        Some(ListCmd::Inc { anime }) => {
            let r = resolve(ctx, &anime)?;
            let cur = match ctx.client.list_entry(&r) {
                Ok(e) => e.episodes.unwrap_or(0.0).floor() as i64,
                Err(e) if api::is_code(&e, "not_in_list") => 0,
                Err(e) => return Err(e),
            };
            let mut patch = json!({ "episodes": cur + 1 });
            if cur == 0 {
                patch["status"] = json!("watching");
            }
            let e = ctx.client.save_entry(&r, &patch)?;
            print_entry_row(&e);
        }
        Some(ListCmd::Rm { anime }) => {
            ctx.client.delete_entry(&resolve(ctx, &anime)?)?;
            println!("Прибрано.");
        }
        Some(ListCmd::Rewatch { anime, cancel }) => {
            ctx.client.rewatch(&resolve(ctx, &anime)?, !cancel)?;
            println!("{}", if cancel { "Повтор скасовано." } else { "Повтор почато." });
        }
    }
    Ok(())
}

fn print_entry_row(e: &ListEntry) {
    let total = short_episodes(&e.anime.episodes).filter(|&t| t > 0).map(|t| t.to_string()).unwrap_or_else(|| "?".into());
    let progress = format!("{}/{total}", e.episodes.map(fmt_num).unwrap_or_else(|| "0".into()));
    let new = e.new_episodes.filter(|&n| n > 0).map(|n| yellow(&format!(" +{n} нових"))).unwrap_or_default();
    let score = e.score.map(|s| format!(" ★{}", fmt_num(s))).unwrap_or_default();
    println!(
        "{}  {}  {}  {}{}{}",
        cyan(&format!("{:>6}", e.anime.ids.mikai.unwrap_or(0))),
        pad(list_status_label(&e.status), 11),
        green(&pad(&progress, 9)),
        bold(e.anime.titles.main()),
        yellow(&score),
        new
    );
}

fn history(ctx: &Ctx, limit: u32) -> Result<()> {
    require_key(ctx)?;
    if ctx.json {
        return print_raw(ctx, "me/history", &[("limit", limit.to_string())]);
    }
    let p = ctx.client.history(1, limit)?;
    for item in &p.items {
        let title = anime_title_of(item);
        let ep = item.get("episode").and_then(Value::as_f64).map(|e| format!("{} еп.", fmt_num(e))).unwrap_or_default();
        let when = ["watchedAt", "updatedAt", "createdAt"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .and_then(parse_time)
            .map(fmt_date)
            .unwrap_or_default();
        println!("{}  {}  {}", dim(&pad(&when, 11)), green(&pad(&ep, 8)), bold(&title));
    }
    Ok(())
}

fn notifications(ctx: &Ctx, filter: &str, read_all: bool) -> Result<()> {
    require_key(ctx)?;
    if read_all {
        ctx.client.mark_all_read()?;
        println!("Усі сповіщення прочитано.");
        return Ok(());
    }
    if ctx.json {
        return print_raw(ctx, "me/notifications", &[("filter", filter.to_string())]);
    }
    let p = ctx.client.notifications(filter, 1, 30)?;
    if p.items.is_empty() {
        println!("{}", dim("Сповіщень немає"));
    }
    for n in &p.items {
        let kind = n.get("type").and_then(Value::as_str).unwrap_or("?");
        let when = n.get("createdAt").and_then(Value::as_str).and_then(parse_time).map(fmt_date).unwrap_or_default();
        let text = match kind {
            "newEpisode" => {
                let ne = n.get("newEpisode").cloned().unwrap_or_default();
                let eps = ne
                    .get("episodes")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_f64).map(fmt_num).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                format!("{} — нові серії {eps}", anime_title_of(&ne))
            }
            "system" => n.pointer("/system/text").and_then(Value::as_str).unwrap_or("").to_string(),
            other => other.to_string(),
        };
        println!("{}  {text}", dim(&pad(&when, 11)));
    }
    Ok(())
}

fn anime_title_of(v: &Value) -> String {
    v.get("anime")
        .and_then(|a| serde_json::from_value::<AnimeShort>(a.clone()).ok())
        .map(|a| a.titles.main().to_string())
        .unwrap_or_else(|| "?".into())
}

// ---------- time ----------

pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

pub fn fmt_date(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%d.%m %H:%M").to_string()
}
