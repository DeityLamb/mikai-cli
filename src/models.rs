//! Typed views over the Mikai API responses.
//!
//! Every field is optional or defaulted: the API promises to add fields and enum
//! values at any time, so nothing here may fail on unknown or missing data.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Ids {
    pub mikai: Option<i64>,
    pub slug: Option<String>,
    pub mal: Option<i64>,
    pub al: Option<i64>,
    pub hikka: Option<String>,
    pub imdb: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Titles {
    pub ua: Option<String>,
    pub english: Option<String>,
    pub original: Option<String>,
    pub synonyms: Vec<String>,
}

impl Titles {
    pub fn main(&self) -> &str {
        self.ua
            .as_deref()
            .or(self.english.as_deref())
            .or(self.original.as_deref())
            .unwrap_or("?")
    }

    /// Original/english title when it differs from the main one.
    pub fn secondary(&self) -> Option<&str> {
        let main = self.main();
        self.original
            .as_deref()
            .or(self.english.as_deref())
            .filter(|t| *t != main)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NamedUa {
    pub name: String,
    pub ua: Option<String>,
}

impl NamedUa {
    pub fn label(&self) -> &str {
        self.ua.as_deref().unwrap_or(&self.name)
    }
}

/// Anime as it appears in lists and inside other objects.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnimeShort {
    pub ids: Ids,
    pub titles: Titles,
    pub format: Option<String>,
    pub status: Option<String>,
    pub season: Option<String>,
    pub year: Option<i64>,
    /// A plain number in short cards.
    pub episodes: Option<Value>,
    pub is_adult: bool,
}

impl AnimeShort {
    pub fn reference(&self) -> Option<String> {
        self.ids.mikai.map(|id| id.to_string())
    }
}

pub fn anime_url(ids: &Ids) -> Option<String> {
    let id = ids.mikai?;
    Some(match &ids.slug {
        Some(slug) => format!("https://mikai.me/anime/{id}-{slug}"),
        None => format!("https://mikai.me/anime/{id}"),
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EpisodesInfo {
    pub total: Option<i64>,
    pub aired: Option<i64>,
    pub localized: Option<i64>,
    pub duration_min: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Scores {
    pub mikai: Option<f64>,
    pub mikai_count: Option<i64>,
    pub mal: Option<f64>,
    pub mal_count: Option<i64>,
    pub hikka: Option<f64>,
    pub hikka_count: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AnimeDetail {
    pub ids: Ids,
    pub titles: Titles,
    pub format: Option<String>,
    pub status: Option<String>,
    pub season: Option<String>,
    pub year: Option<i64>,
    pub start_date: Option<String>,
    pub source: Option<String>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub age_rating: Option<String>,
    pub episodes: EpisodesInfo,
    pub scores: Scores,
    pub genres: Vec<NamedUa>,
    pub tags: Vec<NamedUa>,
    pub description: Option<String>,
    pub licensed: bool,
    pub license: Option<Value>,
    pub releases: Vec<Release>,
}

impl AnimeDetail {
    pub fn url(&self) -> Option<String> {
        anime_url(&self.ids)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Team {
    pub id: Option<i64>,
    pub slug: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Release {
    pub id: String,
    /// `voice` or `sub`.
    pub kind: String,
    pub is_collab: bool,
    pub teams: Vec<Team>,
    pub episodes_count: Option<i64>,
    pub last_episode: Option<f64>,
    pub updated_at: Option<String>,
    pub episodes: Vec<Episode>,
}

impl Release {
    pub fn team_names(&self) -> String {
        let names: Vec<&str> = self.teams.iter().map(|t| t.name.as_str()).collect();
        if names.is_empty() {
            "?".into()
        } else {
            names.join(" × ")
        }
    }

    pub fn kind_label(&self) -> &str {
        kind_label(&self.kind)
    }

    /// Does `query` name this release: its id, a team id, slug or name.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.id.to_lowercase() == q
            || self.teams.iter().any(|t| {
                t.name.to_lowercase().contains(&q)
                    || t.slug.as_deref().is_some_and(|s| s.to_lowercase() == q)
                    || t.id.is_some_and(|id| id.to_string() == q)
            })
    }
}

pub fn kind_label(kind: &str) -> &str {
    match kind {
        "voice" => "озвучення",
        "sub" => "субтитри",
        other => other,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Episode {
    pub number: f64,
    pub label: Option<String>,
    pub added_at: Option<String>,
    pub sources: Vec<Source>,
}

impl Episode {
    pub fn label(&self) -> String {
        self.label.clone().unwrap_or_else(|| fmt_num(self.number))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    pub provider: String,
    pub embed_url: String,
    pub added_at: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Player {
    pub anime: AnimeShort,
    pub licensed: bool,
    pub license: Option<Value>,
    pub releases: Vec<Release>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScheduleItem {
    pub anime: AnimeShort,
    pub episode: Option<f64>,
    pub airing: Option<chrono::DateTime<chrono::Utc>>,
    pub finished: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Update {
    pub anime: AnimeShort,
    pub teams: Vec<Team>,
    pub is_collab: bool,
    pub team_name: Option<String>,
    pub release_id: Option<String>,
    pub kind: String,
    pub provider: Option<String>,
    pub episode: Option<f64>,
    pub embed_url: Option<String>,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListEntry {
    pub anime: AnimeShort,
    pub status: String,
    pub score: Option<f64>,
    pub episodes: Option<f64>,
    pub new_episodes: Option<i64>,
    pub rewatches: Option<i64>,
    pub rewatching: bool,
    pub note: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

pub const LIST_STATUSES: [&str; 5] = ["watching", "planned", "completed", "paused", "dropped"];

pub fn list_status_label(status: &str) -> &str {
    match status {
        "planned" => "заплановано",
        "watching" => "дивлюсь",
        "completed" => "переглянуто",
        "paused" => "відкладено",
        "dropped" => "покинуто",
        other => other,
    }
}

pub fn anime_status_label(status: &str) -> &str {
    match status {
        "finished" => "вийшло",
        "ongoing" => "онгоїнг",
        "announce" => "анонс",
        "cancelled" => "скасовано",
        "break" => "перерва",
        other => other,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct User {
    pub id: Option<i64>,
    pub username: Option<String>,
    pub name: Option<String>,
}

impl User {
    pub fn label(&self) -> String {
        match (&self.name, &self.username) {
            (Some(n), Some(u)) if n != u => format!("{n} (@{u})"),
            (_, Some(u)) => format!("@{u}"),
            (Some(n), None) => n.clone(),
            _ => "?".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Me {
    pub authenticated: bool,
    pub level: Option<String>,
    pub user: Option<User>,
    pub rate_limit_per_min: Option<i64>,
    pub requests_today: Option<i64>,
    pub requests_total: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceStart {
    pub device_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceToken {
    pub key: String,
    pub level: Option<String>,
    pub user: Option<User>,
}

/// `12` instead of `12.0`, but keeps `1.5`.
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Episode count of a short card, which comes as a bare number.
pub fn short_episodes(v: &Option<Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::Object(o)) => o.get("total").and_then(Value::as_i64),
        _ => None,
    }
}
