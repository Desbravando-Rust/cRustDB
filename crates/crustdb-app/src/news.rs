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

const TITLE_MAX: usize = 120;
const URL_MAX: usize = 2048;
/// Response cap for the small JSON feeds (ureq's default is 10 MiB).
const BODY_MAX: u64 = 256 * 1024;

/// Untrusted text for display: tab/CR/LF become spaces, other controls and bidi
/// overrides/isolates are removed, then it is cut to `max` chars (plus `…`).
fn clean(s: &str, max: usize) -> String {
    let s: String = s
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{061C}' => None,
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", s[..end].trim_end()),
        None => s.to_owned(),
    }
}

/// `YYYY-MM-DDT...` shape check; anything else counts as "no date".
fn looks_like_date(d: &str) -> bool {
    let b = d.as_bytes();
    b.len() >= 11
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'T'
}

/// Blog JSON Feed: newest item by `date_published` (first on ties) with a title and an
/// `https://` `url` (max 2048, no whitespace/controls); text fields are sanitized, title cut
/// to 120 and excerpt (`summary`, else `content_text`) to 240 chars.
pub fn parse_blog(body: &str) -> Option<Post> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let str_of =
        |i: &serde_json::Value, k: &str| i.get(k).and_then(|s| s.as_str()).map(str::to_owned);
    v.get("items")?
        .as_array()?
        .iter()
        .filter_map(|i| {
            let url = str_of(i, "url")?;
            // Untrusted remote feed: egui would open any scheme.
            if !url.starts_with("https://")
                || url.len() > URL_MAX
                || url.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                return None;
            }
            let title = clean(&str_of(i, "title")?, TITLE_MAX);
            if title.is_empty() {
                return None;
            }
            let text = str_of(i, "summary")
                .filter(|s| !s.trim().is_empty())
                .or_else(|| str_of(i, "content_text"))
                .unwrap_or_default();
            let excerpt = clean(&text, EXCERPT_MAX);
            // ponytail: plain string compare; wrong for mixed UTC offsets or out-of-range values
            // (e.g. month 99). Parse RFC 3339 if a time crate ever becomes a dependency.
            let date = str_of(i, "date_published").filter(|d| looks_like_date(d));
            Some((
                date,
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
    match r.and_then(|mut r| r.body_mut().with_config().limit(BODY_MAX).read_to_string()) {
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
                repaint();
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
                repaint();
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
            {"title":"New","url":"https://x/new","content_text":"  body  ","date_published":"2026-02-01T00:00:00Z"}]}"#;

        // Act
        let post = parse_blog(body);

        // Assert
        assert_eq!(
            post,
            Some(Post {
                title: "New".into(),
                url: "https://x/new".into(),
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
        let body = r#"{"items":[{"title":"X","url":"javascript:alert(1)"},{"title":"Y","url":"file:///etc/passwd"},{"title":"Z","url":"http://x/z"}]}"#;

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

    #[test]
    fn parse_blog_sanitizes_controls_and_bidi() {
        // Arrange
        let body = r#"{"items":[{"title":"A\nB\u0007C\u202eD","url":"https://x/a","summary":"x\ty\r\nz\u2066w"}]}"#;

        // Act
        let post = parse_blog(body).unwrap();

        // Assert
        assert_eq!(post.title, "A BCD");
        assert_eq!(post.excerpt, "x y  zw");
    }

    #[test]
    fn parse_blog_caps_title_and_drops_long_or_dirty_url() {
        // Arrange
        let long_title = "t".repeat(500);
        let long_url = format!("https://x/{}", "a".repeat(URL_MAX));
        let body = format!(
            r#"{{"items":[{{"title":"T","url":"{long_url}"}},{{"title":"T","url":"https://x/a b"}},{{"title":"T","url":"https://x/\u0000"}},{{"title":"{long_title}","url":"https://x/ok"}}]}}"#
        );

        // Act
        let post = parse_blog(&body).unwrap();

        // Assert
        assert_eq!(post.url, "https://x/ok");
        assert_eq!(post.title.chars().count(), TITLE_MAX + 1);
    }

    #[test]
    fn parse_blog_ignores_garbage_dates() {
        // Arrange
        let body = r#"{"items":[
            {"title":"Garbage","url":"https://x/g","date_published":"zzzz"},
            {"title":"Old","url":"https://x/o","date_published":"2026-01-01T00:00:00Z"},
            {"title":"New","url":"https://x/n","date_published":"2026-03-01T10:00:00+02:00"}]}"#;

        // Act
        let post = parse_blog(body).unwrap();

        // Assert
        assert_eq!(post.title, "New");
    }

    #[test]
    fn looks_like_date_checks_shape() {
        // Act & Assert
        assert!(looks_like_date("2026-11-10T12:00:00Z"));
        assert!(!looks_like_date("zzzz"));
        assert!(!looks_like_date("2026-11-10"));
    }
}
