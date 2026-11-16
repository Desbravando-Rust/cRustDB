//! Opt-out network egress (ADR 0012): latest release tag, book feed and blog feed.

use std::sync::{Arc, OnceLock};

pub const RELEASES_URL: &str = "https://github.com/Desbravando-Rust/cRustDB/releases/latest";

/// Results filled once by the background thread; the UI only reads.
#[derive(Default)]
pub struct News {
    pub latest: OnceLock<String>,
    pub chapter: OnceLock<Chapter>,
    pub post: OnceLock<Post>,
}

/// Latest blog post (ADR 0012 addendum); `excerpt` is empty when the feed has none.
#[derive(Debug, PartialEq, Eq)]
pub struct Post {
    pub title: String,
    pub url: String,
    pub excerpt: String,
}

const EXCERPT_MAX: usize = 240;

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

/// JSON Feed `items[]`; items without an `https://` `url` are dropped.
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
            // Untrusted remote feed: egui would open any scheme.
            if !url.starts_with("https://") {
                return None;
            }
            Some(Chapter { title, url })
        })
        .collect()
}

/// Blog JSON Feed: newest item by `date_published` (first on ties) with a title and an
/// http(s) `url`; excerpt = `summary` or `content_text`, cut to 240 chars.
pub fn parse_blog(body: &str) -> Option<Post> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let str_of =
        |i: &serde_json::Value, k: &str| i.get(k).and_then(|s| s.as_str()).map(str::to_owned);
    v.get("items")?
        .as_array()?
        .iter()
        .filter_map(|i| {
            let (title, url) = (str_of(i, "title")?, str_of(i, "url")?);
            // Untrusted remote feed: egui would open any scheme.
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return None;
            }
            let text = str_of(i, "summary")
                .filter(|s| !s.trim().is_empty())
                .or_else(|| str_of(i, "content_text"))
                .unwrap_or_default();
            let text = text.trim();
            let excerpt = match text.char_indices().nth(EXCERPT_MAX) {
                Some((end, _)) => format!("{}…", text[..end].trim_end()),
                None => text.to_owned(),
            };
            Some((
                str_of(i, "date_published"),
                Post {
                    title,
                    url,
                    excerpt,
                },
            ))
        })
        .rev() // max_by_key keeps the last max: reversed, ties favor the first item
        .max_by_key(|(date, _)| date.clone())
        .map(|(_, p)| p)
}

const BLOG_URL: &str = "https://desbravandorust.com.br/blog.json";
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

/// `CRUSTDB_OFFLINE` counts as set unless empty or `0`.
fn offline_from(v: Option<&str>) -> bool {
    v.is_some_and(|v| !v.is_empty() && v != "0")
}

/// One background thread, no data sent; skipped when `CRUSTDB_OFFLINE` is set (not empty or `0`). Failures keep the static fallback.
pub fn spawn(news: Arc<News>, repaint: impl Fn() + Send + 'static) {
    if offline_from(std::env::var("CRUSTDB_OFFLINE").ok().as_deref()) {
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
            if let Some(p) = fetch(&agent, BLOG_URL).and_then(|b| parse_blog(&b)) {
                let _ = news.post.set(p);
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
    fn offline_from_ignores_empty_and_zero() {
        // Act & Assert
        assert!(!offline_from(None));
        assert!(!offline_from(Some("")));
        assert!(!offline_from(Some("0")));
        assert!(offline_from(Some("1")));
    }

    #[test]
    fn parse_feed_reads_title_and_url() {
        // Arrange
        let body = r#"{"items":[{"title":"Ch 1","url":"https://x/1"},{"title":"No url"},{"title":"Bad","url":"javascript:x"},{"title":"Http","url":"http://x/2"}]}"#;

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

    #[test]
    fn parse_blog_picks_newest_with_excerpt() {
        // Arrange
        let body = r#"{"items":[
            {"title":"Old","url":"https://x/old","summary":"s","date_published":"2026-01-01T00:00:00Z"},
            {"title":"New","url":"http://x/new","content_text":"  body  ","date_published":"2026-02-01T00:00:00Z"}]}"#;

        // Act
        let post = parse_blog(body);

        // Assert
        assert_eq!(
            post,
            Some(Post {
                title: "New".into(),
                url: "http://x/new".into(),
                excerpt: "body".into()
            })
        );
    }

    #[test]
    fn parse_blog_tolerates_missing_fields_and_first_wins_without_dates() {
        // Arrange
        let body = r#"{"items":[{"url":"https://x/0"},{"title":"A","url":"https://x/a"},{"title":"B","url":"https://x/b"}]}"#;

        // Act
        let post = parse_blog(body).unwrap();

        // Assert
        assert_eq!((post.title.as_str(), post.excerpt.as_str()), ("A", ""));
    }

    #[test]
    fn parse_blog_drops_bad_scheme() {
        // Arrange
        let body = r#"{"items":[{"title":"X","url":"javascript:alert(1)"},{"title":"Y","url":"file:///etc/passwd"}]}"#;

        // Act & Assert
        assert_eq!(parse_blog(body), None);
    }

    #[test]
    fn parse_blog_truncates_on_char_boundary() {
        // Arrange
        let long = "é".repeat(300);
        let body = format!(r#"{{"items":[{{"title":"T","url":"https://x","summary":"{long}"}}]}}"#);

        // Act
        let post = parse_blog(&body).unwrap();

        // Assert
        assert_eq!(post.excerpt.chars().count(), EXCERPT_MAX + 1);
        assert!(post.excerpt.ends_with('…'));
    }

    #[test]
    fn parse_blog_empty_or_invalid_is_none() {
        // Act & Assert
        assert_eq!(parse_blog(r#"{"items":[]}"#), None);
        assert_eq!(parse_blog("not json"), None);
        assert_eq!(parse_blog("{}"), None);
    }
}
