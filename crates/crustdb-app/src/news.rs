//! Opt-out network egress (ADR 0012): latest release tag and book feed.

use std::sync::{Arc, OnceLock};

pub const RELEASES_URL: &str = "https://github.com/Desbravando-Rust/cRustDB/releases/latest";

/// Results filled once by the background thread; the UI only reads.
#[derive(Default)]
pub struct News {
    pub latest: OnceLock<String>,
    pub chapter: OnceLock<Chapter>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Chapter {
    pub title: String,
    pub url: String,
}

fn version(s: &str) -> Option<Vec<u64>> {
    s.trim_start_matches('v')
        .split('.')
        .map(|p| p.parse().ok())
        .collect()
}

/// True when `tag` (e.g. `v0.2.0`) is a higher dotted version than `current`.
pub fn is_newer(current: &str, tag: &str) -> bool {
    matches!((version(current), version(tag)), (Some(c), Some(t)) if t > c)
}

pub fn parse_latest_tag(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(v.get("tag_name")?.as_str()?.to_owned())
}

/// JSON Feed `items[]`; items without a `url` are dropped.
pub fn parse_feed(body: &str) -> Vec<Chapter> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return vec![];
    };
    let items = v.get("items").and_then(|i| i.as_array());
    items
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let title = i.get("title")?.as_str()?.to_owned();
            let url = i.get("url")?.as_str()?.to_owned();
            Some(Chapter { title, url })
        })
        .collect()
}

const RELEASE_API: &str = "https://api.github.com/repos/Desbravando-Rust/cRustDB/releases/latest";
const FEED_URL: &str = "https://desbravandorust.com.br/feed.json";

fn fetch(agent: &ureq::Agent, url: &str) -> Option<String> {
    let r = agent.get(url).call();
    match r.and_then(|mut r| r.body_mut().read_to_string()) {
        Ok(b) => Some(b),
        Err(e) => {
            tracing::debug!("news fetch {url} failed: {e}");
            None
        }
    }
}

/// One background thread, no data sent; skipped when `CRUSTDB_OFFLINE` is set. Failures keep the static fallback.
pub fn spawn(news: Arc<News>, repaint: impl Fn() + Send + 'static) {
    if std::env::var_os("CRUSTDB_OFFLINE").is_some() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("crustdb-news".into())
        .spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(5)))
                .user_agent(concat!("cRustDB/", env!("CARGO_PKG_VERSION")))
                .build()
                .into();
            if let Some(tag) = fetch(&agent, RELEASE_API).and_then(|b| parse_latest_tag(&b)) {
                let _ = news.latest.set(tag);
            }
            let chapters = fetch(&agent, FEED_URL)
                .map(|b| parse_feed(&b))
                .unwrap_or_default();
            // ponytail: clock nanos instead of a rand dep; fine for picking a banner chapter.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos() as usize);
            let pick = nanos.checked_rem(chapters.len()).unwrap_or(0);
            if let Some(c) = chapters.into_iter().nth(pick) {
                let _ = news.chapter.set(c);
            }
            repaint();
        });
    if let Err(e) = spawned {
        tracing::debug!("news thread not started: {e}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn is_newer_detects_higher_tag() {
        // Act & Assert
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("0.2.0", "v0.2.0"));
        assert!(!is_newer("0.2.0", "v0.1.9"));
        assert!(!is_newer("0.1.0", "nightly"));
    }

    #[test]
    fn parse_latest_tag_reads_tag_name() {
        // Act & Assert
        assert_eq!(
            parse_latest_tag(r#"{"id":1,"tag_name":"v0.3.0"}"#).as_deref(),
            Some("v0.3.0")
        );
        assert_eq!(parse_latest_tag("not json"), None);
    }

    #[test]
    fn parse_feed_reads_title_and_url() {
        // Arrange
        let body = r#"{"items":[{"title":"Ch 1","url":"https://x/1"},{"title":"No url"}]}"#;

        // Act
        let chapters = parse_feed(body);

        // Assert
        assert_eq!(
            chapters,
            vec![Chapter {
                title: "Ch 1".into(),
                url: "https://x/1".into()
            }]
        );
        assert!(parse_feed(r#"{"items":[]}"#).is_empty());
    }
}
