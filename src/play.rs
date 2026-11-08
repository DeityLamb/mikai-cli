//! Playing episodes outside the browser.
//!
//! The API only gives provider embed pages. Some providers put a plain HLS
//! link into the page's player config; for those the stream can go straight to
//! mpv. Providers that encrypt or otherwise protect their streams are left to
//! the browser: this module does not try to get around that.

use std::{
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};

/// Providers whose embed page exposes a plain stream link.
const MPV_PROVIDERS: &[&str] = &["ashdi"];

const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36";

pub fn supports_mpv(provider: &str) -> bool {
    MPV_PROVIDERS.iter().any(|p| p.eq_ignore_ascii_case(provider))
}

pub fn mpv_available() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let p = dir.join("mpv");
            p.is_file() || p.with_extension("exe").is_file()
        })
    })
}

pub struct Stream {
    pub url: String,
    pub referer: String,
    pub subtitles: Vec<String>,
}

/// Reads the stream link out of a provider embed page.
pub fn stream(embed_url: &str) -> Result<Stream> {
    let http = reqwest::blocking::Client::builder()
        .user_agent(BROWSER_UA)
        .timeout(Duration::from_secs(20))
        .build()?;
    let html = http
        .get(embed_url)
        .header("Referer", "https://mikai.me/")
        .send()
        .and_then(|r| r.error_for_status())
        .context("плеєр не відповідає")?
        .text()?;
    let url = js_field(&html, "file")
        .filter(|u| u.starts_with("http"))
        .context("у сторінці плеєра немає прямого посилання на відео")?;
    let subtitles = js_field(&html, "subtitle")
        .map(|s| {
            s.split(',')
                .map(|item| item.rsplit(']').next().unwrap_or(item).trim().to_string())
                .filter(|u| u.starts_with("http"))
                .collect()
        })
        .unwrap_or_default();
    let origin = embed_url.splitn(4, '/').take(3).collect::<Vec<_>>().join("/") + "/";
    Ok(Stream { url, referer: origin, subtitles })
}

/// Value of `name:'…'` / `name: "…"` in an inline player config.
fn js_field(html: &str, name: &str) -> Option<String> {
    let mut rest = html;
    while let Some(i) = rest.find(name) {
        let after = rest[i + name.len()..].trim_start_matches(['"', '\'']).trim_start();
        if let Some(after) = after.strip_prefix(':') {
            let after = after.trim_start();
            if let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let body = &after[1..];
                if let Some(end) = body.find(q) {
                    return Some(body[..end].to_string());
                }
            }
        }
        rest = &rest[i + name.len()..];
    }
    None
}

/// Starts mpv detached from the terminal; it keeps playing after mikai exits.
pub fn launch_mpv(stream: &Stream, title: &str) -> Result<()> {
    if !mpv_available() {
        bail!("mpv не встановлено");
    }
    let mut cmd = Command::new("mpv");
    cmd.arg(format!("--referrer={}", stream.referer))
        .arg(format!("--force-media-title={title}"))
        .arg("--no-terminal");
    for s in &stream.subtitles {
        cmd.arg(format!("--sub-file={s}"));
    }
    cmd.arg(&stream.url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().context("не вдалося запустити mpv")?;
    Ok(())
}
