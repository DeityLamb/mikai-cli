//! Thin blocking client for the Mikai public API (`/public/v1`).

use std::{fmt, thread, time::Duration};

use anyhow::{Context, Result};
use reqwest::{Method, StatusCode, blocking::RequestBuilder};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::models::*;

pub const BASE_URL: &str = "https://api.mikai.me/public/v1";
pub const USER_AGENT: &str = concat!(
    "mikai-cli/",
    env!("CARGO_PKG_VERSION"),
    " (+https://mikaicli.deitylamb.dev)"
);

/// An `ok: false` answer. Match on `code`, never on `message`.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// HTTP status of the answer.
    #[allow(dead_code)]
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hint = match self.code.as_str() {
            "api_key_required" => " (увійдіть: mikai login)",
            "invalid_api_key" | "api_key_expired" => " (увійдіть знову: mikai login)",
            "insufficient_scope" => " (ключу бракує рівня доступу: mikai login --level write)",
            _ => "",
        };
        write!(f, "{} [{}]{hint}", self.message, self.code)
    }
}

impl std::error::Error for ApiError {}

/// Is `err` an API error with this `code`.
pub fn is_code(err: &anyhow::Error, code: &str) -> bool {
    err.downcast_ref::<ApiError>().is_some_and(|e| e.code == code)
}

#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: Option<u64>,
    pub page: Option<u64>,
    pub pages: Option<u64>,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::blocking::Client,
    base: String,
    key: Option<String>,
}

pub type Query = Vec<(&'static str, String)>;

impl Client {
    pub fn new(key: Option<String>) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()?;
        let base = std::env::var("MIKAI_API_URL").unwrap_or_else(|_| BASE_URL.to_string());
        Ok(Self { http, base, key })
    }

    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }

    pub fn set_key(&mut self, key: Option<String>) {
        self.key = key;
    }

    fn build(&self, method: &Method, path: &str, query: &[(&str, String)], body: Option<&Value>) -> RequestBuilder {
        let url = format!("{}/{}", self.base, path.trim_start_matches('/'));
        let mut req = self.http.request(method.clone(), url).query(query);
        if let Some(key) = &self.key {
            req = req.bearer_auth(key);
        }
        if let Some(body) = body {
            req = req.json(body);
        }
        req
    }

    /// Sends a request and returns the whole envelope (`ok`, `result`, `total`…).
    pub fn raw(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<&Value>) -> Result<Value> {
        let mut wait = Duration::from_secs(1);
        for attempt in 0..4 {
            let resp = self
                .build(&method, path, query, body)
                .send()
                .with_context(|| format!("не вдалося зʼєднатися з {}", self.base))?;
            let status = resp.status();
            if status == StatusCode::TOO_MANY_REQUESTS && attempt < 3 {
                let retry = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or(wait);
                thread::sleep(retry.max(wait));
                wait *= 2;
                continue;
            }
            let text = resp.text()?;
            let value: Value = serde_json::from_str(&text).map_err(|_| ApiError {
                status: status.as_u16(),
                code: "bad_response".into(),
                message: format!("HTTP {status}: {}", text.chars().take(200).collect::<String>()),
            })?;
            if value.get("ok").and_then(Value::as_bool) == Some(true) {
                return Ok(value);
            }
            let err = value.get("error").cloned().unwrap_or_default();
            return Err(ApiError {
                status: status.as_u16(),
                code: err["code"].as_str().unwrap_or("unknown").to_string(),
                message: err["message"].as_str().unwrap_or("невідома помилка").to_string(),
            }
            .into());
        }
        unreachable!()
    }

    fn call<T: DeserializeOwned>(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<&Value>) -> Result<T> {
        let mut env = self.raw(method, path, query, body)?;
        let result = env.get_mut("result").map(Value::take).unwrap_or(Value::Null);
        serde_json::from_value(result).with_context(|| format!("неочікувана відповідь від {path}"))
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        self.call(Method::GET, path, query, None)
    }

    fn page<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<Page<T>> {
        let mut env = self.raw(Method::GET, path, query, None)?;
        let items = serde_json::from_value(env["result"].take()).with_context(|| format!("неочікувана відповідь від {path}"))?;
        Ok(Page {
            items,
            total: env["total"].as_u64(),
            page: env["page"].as_u64(),
            pages: env["pages"].as_u64(),
        })
    }

    // ---- catalogue ----

    pub fn search(&self, query: &[(&'static str, String)]) -> Result<Page<AnimeShort>> {
        self.page("anime", query)
    }

    pub fn anime(&self, r: &str) -> Result<AnimeDetail> {
        self.get(&format!("anime/{}", enc(r)), &[])
    }

    pub fn random(&self) -> Result<AnimeDetail> {
        self.get("anime/random", &[])
    }

    pub fn player(&self, r: &str) -> Result<Player> {
        self.get(&format!("anime/{}/player", enc(r)), &[])
    }

    /// Weekday (`monday`…) → items.
    pub fn schedule(&self) -> Result<std::collections::BTreeMap<String, Vec<ScheduleItem>>> {
        self.get("schedule", &[])
    }

    pub fn updates(&self, page: u32, limit: u32) -> Result<Page<Update>> {
        self.page("updates", &[("page", page.to_string()), ("limit", limit.to_string())])
    }

    /// Voice and subtitle teams, 50 per page.
    pub fn teams(&self, page: u32) -> Result<Page<Team>> {
        self.page("teams", &[("page", page.to_string()), ("limit", "50".into())])
    }

    // ---- account ----

    pub fn me(&self) -> Result<Me> {
        self.get("me", &[])
    }

    pub fn my_list(&self, status: Option<&str>, page: u32, limit: u32) -> Result<Page<ListEntry>> {
        let mut q = vec![("page", page.to_string()), ("limit", limit.to_string())];
        if let Some(s) = status {
            q.push(("status", s.to_string()));
        }
        self.page("me/list", &q)
    }

    pub fn list_entry(&self, r: &str) -> Result<ListEntry> {
        self.get(&format!("me/list/{}", enc(r)), &[])
    }

    /// Only the fields present in `patch` change.
    pub fn save_entry(&self, r: &str, patch: &Value) -> Result<ListEntry> {
        self.call(Method::PUT, &format!("me/list/{}", enc(r)), &[], Some(patch))
    }

    pub fn delete_entry(&self, r: &str) -> Result<()> {
        self.raw(Method::DELETE, &format!("me/list/{}", enc(r)), &[], None).map(drop)
    }

    pub fn rewatch(&self, r: &str, start: bool) -> Result<()> {
        let m = if start { Method::POST } else { Method::DELETE };
        self.raw(m, &format!("me/list/{}/rewatch", enc(r)), &[], None).map(drop)
    }

    pub fn history(&self, page: u32, limit: u32) -> Result<Page<Value>> {
        self.page("me/history", &[("page", page.to_string()), ("limit", limit.to_string())])
    }

    pub fn notifications(&self, filter: &str, page: u32, limit: u32) -> Result<Page<Value>> {
        self.page(
            "me/notifications",
            &[("filter", filter.to_string()), ("page", page.to_string()), ("limit", limit.to_string())],
        )
    }

    pub fn mark_all_read(&self) -> Result<()> {
        self.raw(Method::POST, "me/notifications/read", &[], Some(&json!({ "all": true }))).map(drop)
    }

    pub fn delete_key(&self) -> Result<()> {
        self.raw(Method::DELETE, "me/key", &[], None).map(drop)
    }

    // ---- device login ----

    pub fn device_start(&self, client_id: &str, level: &str) -> Result<DeviceStart> {
        self.call(Method::POST, "auth/device", &[], Some(&json!({ "clientId": client_id, "level": level })))
    }

    pub fn device_token(&self, client_id: &str, device_code: &str) -> Result<DeviceToken> {
        self.call(
            Method::POST,
            "auth/device/token",
            &[],
            Some(&json!({ "clientId": client_id, "deviceCode": device_code })),
        )
    }

    pub fn device_cancel(&self, client_id: &str, device_code: &str) -> Result<()> {
        self.raw(
            Method::POST,
            "auth/device/cancel",
            &[],
            Some(&json!({ "clientId": client_id, "deviceCode": device_code })),
        )
        .map(drop)
    }

    // ---- misc ----

    pub fn translit(&self, text: &str, lang: Option<&str>) -> Result<Value> {
        let mut q = vec![("text", text.to_string())];
        if let Some(l) = lang {
            q.push(("lang", l.to_string()));
        }
        self.get("translit", &q)
    }
}

/// Accepts anything a human might paste and turns it into an API `{ref}`:
/// `1272`, `mal:52991`, `slug:…`, or a `https://mikai.me/anime/1272-slug` link.
pub fn normalize_ref(input: &str) -> String {
    let s = input.trim();
    if let Some(rest) = s.split("mikai.me/anime/").nth(1) {
        let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() {
            return id;
        }
    }
    for (host, prefix) in [("myanimelist.net/anime/", "mal"), ("anilist.co/anime/", "al")] {
        if let Some(rest) = s.split(host).nth(1) {
            let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if !id.is_empty() {
                return format!("{prefix}:{id}");
            }
        }
    }
    if let Some(rest) = s.split("hikka.io/anime/").nth(1) {
        let slug = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        return format!("hikka:{slug}");
    }
    s.to_string()
}

fn enc(r: &str) -> String {
    r.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' | ':' => c.to_string(),
            c => {
                let mut buf = [0u8; 4];
                c.encode_utf8(&mut buf).bytes().map(|b| format!("%{b:02X}")).collect()
            }
        })
        .collect()
}
