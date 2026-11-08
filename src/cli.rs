//! Command-line interface: argument definitions and the command handlers.

use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use reqwest::Method;
use serde_json::{Map, Value, json};

use crate::{
    api::{self, Client, normalize_ref},
    config::Config,
    models::*,
    style::{bold, dim, green, yellow},
    view,
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
        Command::Show { anime } => {
            let r = resolve_cli(ctx, &anime)?;
            if ctx.json {
                return print_raw(ctx, &format!("anime/{r}"), &[]);
            }
            let a = ctx.client.anime(&r)?;
            view::print(&view::anime_card(&a, None, false));
            Ok(())
        }
        Command::Random => {
            if ctx.json {
                return print_raw(ctx, "anime/random", &[]);
            }
            view::print(&view::anime_card(&ctx.client.random()?, None, false));
            Ok(())
        }
        Command::Episodes { anime, team, kind } => {
            let r = resolve_cli(ctx, &anime)?;
            episodes(ctx, &r, team, kind)
        }
        Command::Open { anime, episode, team, kind, provider, print } => {
            let r = resolve_cli(ctx, &anime)?;
            open_episode(ctx, &r, episode, team, kind, provider, print)
        }
        Command::Schedule { day } => {
            if ctx.json {
                return print_raw(ctx, "schedule", &[]);
            }
            let day = day.map(|d| view::weekday_key(&d));
            let (lines, _) = view::schedule(ctx.client.schedule()?, day.as_deref(), false);
            view::print(&lines);
            Ok(())
        }
        Command::Updates { limit, page } => {
            if ctx.json {
                return print_raw(ctx, "updates", &[("limit", limit.to_string()), ("page", page.to_string())]);
            }
            view::print(&view::updates(&ctx.client.updates(page, limit)?.items, false));
            Ok(())
        }
        Command::Login { with_key, level, client_id, no_browser } => login(ctx, with_key, &level, client_id, no_browser),
        Command::Logout { local } => logout(ctx, local),
        Command::Whoami => {
            if ctx.json {
                return print_raw(ctx, "me", &[]);
            }
            view::print(&view::me(&ctx.client.me()?));
            Ok(())
        }
        Command::List(a) => list(ctx, a),
        Command::History { limit } => {
            require_key(ctx)?;
            if ctx.json {
                return print_raw(ctx, "me/history", &[("limit", limit.to_string())]);
            }
            view::print(&view::history(&ctx.client.history(1, limit)?.items, false));
            Ok(())
        }
        Command::Notifications { filter, read_all } => {
            require_key(ctx)?;
            if read_all {
                ctx.client.mark_all_read()?;
                println!("Усі сповіщення прочитано.");
                return Ok(());
            }
            if ctx.json {
                return print_raw(ctx, "me/notifications", &[("filter", filter.clone())]);
            }
            view::print(&view::notifications(&ctx.client.notifications(&filter, 1, 30)?.items));
            Ok(())
        }
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
/// ref or a link is treated as a title and resolved to the best search hit,
/// which is returned too.
pub fn resolve(client: &Client, input: &str) -> Result<(String, Option<AnimeShort>)> {
    let r = normalize_ref(input);
    if r.contains(':') || r.parse::<i64>().is_ok() {
        return Ok((r, None));
    }
    let page = client.search(&[("search", r.clone()), ("limit", "1".into())])?;
    let hit = page.items.into_iter().next().with_context(|| format!("аніме «{r}» не знайдено"))?;
    let id = hit.reference().context("у результату немає id")?;
    Ok((id, Some(hit)))
}

fn resolve_cli(ctx: &Ctx, input: &str) -> Result<String> {
    let (r, hit) = resolve(&ctx.client, input)?;
    if let Some(hit) = hit {
        eprintln!("{}", dim(&format!("→ {} (#{r})", hit.titles.main())));
    }
    Ok(r)
}

fn print_raw(ctx: &Ctx, path: &str, q: &[(&str, String)]) -> Result<()> {
    let v = ctx.client.raw(Method::GET, path, q, None)?;
    println!("{}", serde_json::to_string_pretty(&v)?);
    Ok(())
}

pub fn search_query(text: &str, limit: u32, page: u32) -> api::Query {
    let mut q: api::Query = vec![("limit", limit.to_string()), ("page", page.to_string())];
    if !text.is_empty() {
        q.push(("search", text.to_string()));
    }
    q
}

fn search(ctx: &Ctx, a: SearchArgs) -> Result<()> {
    let mut q = search_query(&a.query.join(" "), a.limit, a.page);
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
    let mut lines = view::anime_rows(&page.items, false);
    lines.extend(view::page_footer(page.page, page.pages, page.total));
    view::print(&lines);
    Ok(())
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
    let releases: Vec<Release> = p
        .releases
        .into_iter()
        .filter(|rel| kind.as_deref().is_none_or(|k| rel.kind == k))
        .filter(|rel| team.as_deref().is_none_or(|t| rel.matches(t)))
        .collect();
    if releases.is_empty() {
        bail!("немає релізів за цим фільтром");
    }
    if team.is_none() {
        view::print(&view::releases(&releases, false));
        println!("{}", dim("Серії релізу: mikai episodes <аніме> --team <команда>"));
        return Ok(());
    }
    for rel in &releases {
        println!();
        view::print(&view::episodes(rel, 0.0, None));
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
        None => ctx.client.anime(r)?.url().context("у аніме немає сторінки")?,
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

// ---------- account ----------

fn require_key(ctx: &Ctx) -> Result<()> {
    if !ctx.client.has_key() {
        bail!("потрібен вхід: mikai login");
    }
    Ok(())
}

pub const NO_CLIENT_ID: &str = "немає clientId застосунку.\n\
     Увійдіть готовим ключем: створіть його на https://mikai.me/user/developers і виконайте\n  \
     mikai login --with-key mk_…\n\
     або передайте clientId: mikai login --client-id mkc_…";

/// Checks `key` and stores it. Returns the user label.
pub fn save_key(client: &mut Client, config: &mut Config, key: String) -> Result<(String, Option<String>)> {
    client.set_key(Some(key.clone()));
    let me = client.me().context("ключ не підійшов")?;
    if !me.authenticated {
        client.set_key(None);
        bail!("ключ не підійшов");
    }
    config.key = Some(key);
    config.level = me.level.clone();
    config.user = me.user.as_ref().map(User::label);
    config.save()?;
    Ok((config.user.clone().unwrap_or_else(|| "?".into()), me.level))
}

/// Device login: shows the link through `show`, polls until confirmed.
pub fn device_login(
    client: &Client,
    client_id: &str,
    level: &str,
    open_browser: bool,
    show: impl FnOnce(&str),
) -> Result<DeviceToken> {
    let start = client.device_start(client_id, level)?;
    show(&start.verification_url);
    if open_browser {
        let _ = open::that_detached(&start.verification_url);
    }
    let deadline = Instant::now() + Duration::from_secs(start.expires_in.max(60));
    let mut interval = start.interval.max(1);
    loop {
        if Instant::now() > deadline {
            bail!("посилання протухло, спробуйте ще раз");
        }
        thread::sleep(Duration::from_secs(interval));
        match client.device_token(client_id, &start.device_code) {
            Ok(t) => return Ok(t),
            Err(e) if api::is_code(&e, "authorization_pending") => {}
            Err(e) if api::is_code(&e, "slow_down") => interval += 5,
            Err(e) if api::is_code(&e, "access_denied") => bail!("вхід відхилено"),
            Err(e) if api::is_code(&e, "expired_token") => bail!("посилання протухло, спробуйте ще раз"),
            Err(e) => {
                let _ = client.device_cancel(client_id, &start.device_code);
                return Err(e);
            }
        }
    }
}

fn login(ctx: &mut Ctx, with_key: Option<String>, level: &str, client_id: Option<String>, no_browser: bool) -> Result<()> {
    if let Some(key) = with_key {
        let (user, level) = save_key(&mut ctx.client, &mut ctx.config, key)?;
        println!("{} Увійшли як {} (рівень {})", green("✓"), bold(&user), level.as_deref().unwrap_or("?"));
        return Ok(());
    }
    let client_id = client_id.or_else(|| ctx.config.client_id()).context(NO_CLIENT_ID)?;
    let token = device_login(&ctx.client, &client_id, level, !no_browser, |url| {
        println!("Підтвердіть вхід у браузері:\n\n  {}\n", bold(url));
        eprintln!("{}", dim("Чекаю на підтвердження… (Ctrl+C — скасувати)"));
    })?;
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

/// Builds a `PUT /me/list` body. `score == 0` clears the score.
pub fn entry_patch(status: Option<String>, episodes: Option<i64>, score: Option<f64>, note: Option<String>) -> Value {
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
    Value::Object(patch)
}

/// Current entry or `None` when the anime is in no list.
pub fn entry_of(client: &Client, r: &str) -> Result<Option<ListEntry>> {
    match client.list_entry(r) {
        Ok(e) => Ok(Some(e)),
        Err(e) if api::is_code(&e, "not_in_list") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Marks one more episode watched; starts `watching` for new titles.
pub fn increment(client: &Client, r: &str) -> Result<ListEntry> {
    let cur = entry_of(client, r)?;
    let n = cur.as_ref().and_then(|e| e.episodes).unwrap_or(0.0).floor() as i64;
    let mut patch = json!({ "episodes": n + 1 });
    if cur.is_none() {
        patch["status"] = json!("watching");
    }
    client.save_entry(r, &patch)
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
            let mut lines = view::list_entries(&p.items, false);
            lines.extend(view::page_footer(p.page, p.pages, p.total));
            view::print(&lines);
        }
        Some(ListCmd::Get { anime }) => {
            let r = resolve_cli(ctx, &anime)?;
            if ctx.json {
                return print_raw(ctx, &format!("me/list/{r}"), &[]);
            }
            match entry_of(&ctx.client, &r)? {
                Some(e) => view::print(&[view::list_entry_row(&e, 0, false)]),
                None => println!("{}", dim("Немає в списках")),
            }
        }
        Some(ListCmd::Set { anime, status, episodes, score, note }) => {
            let patch = entry_patch(status, episodes, score, note);
            if patch.as_object().is_some_and(Map::is_empty) {
                bail!("нічого змінювати: вкажіть --status, --episodes, --score чи --note");
            }
            let r = resolve_cli(ctx, &anime)?;
            let e = ctx.client.save_entry(&r, &patch)?;
            view::print(&[view::list_entry_row(&e, 0, false)]);
        }
        Some(ListCmd::Inc { anime }) => {
            let r = resolve_cli(ctx, &anime)?;
            let e = increment(&ctx.client, &r)?;
            view::print(&[view::list_entry_row(&e, 0, false)]);
        }
        Some(ListCmd::Rm { anime }) => {
            ctx.client.delete_entry(&resolve_cli(ctx, &anime)?)?;
            println!("Прибрано.");
        }
        Some(ListCmd::Rewatch { anime, cancel }) => {
            ctx.client.rewatch(&resolve_cli(ctx, &anime)?, !cancel)?;
            println!("{}", if cancel { "Повтор скасовано." } else { "Повтор почато." });
        }
    }
    Ok(())
}
