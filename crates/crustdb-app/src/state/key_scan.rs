//! Incremental keyspace scan of a store workspace: one batch in flight, deduplicated entries.
use std::collections::HashSet;

use crustdb_core::store::keyspace::EntryKey;
use crustdb_core::store::{KeyEntry, ScanPage};

use crate::runtime::{Command, RequestId, WorkspaceId};

#[derive(Debug, Default)]
pub struct KeyScan {
    pub entries: Vec<KeyEntry>,
    seen: HashSet<EntryKey>,
    pattern: Option<String>,
    cursor: Option<String>,
    pending: Option<RequestId>,
    done: bool,
}

impl KeyScan {
    pub fn new(pattern: Option<String>) -> Self {
        Self {
            pattern,
            ..Self::default()
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The command for the next batch; `None` while one is in flight or when the scan is complete.
    pub fn next(&mut self, ws: WorkspaceId, req: RequestId) -> Option<Command> {
        if self.done || self.pending.is_some() {
            return None;
        }
        self.pending = Some(req);
        Some(Command::ScanKeys {
            ws,
            req,
            cursor: self.cursor.clone(),
            pattern: self.pattern.clone(),
        })
    }

    /// Adds the batch answering `req`, skipping keys already seen (scans may repeat keys).
    /// Answers to other requests are ignored.
    pub fn apply(&mut self, req: RequestId, page: ScanPage) {
        if self.pending != Some(req) {
            return;
        }
        self.pending = None;
        self.done = page.next.is_none();
        self.cursor = page.next;
        for e in page.entries {
            if self.seen.insert(e.key.clone()) {
                self.entries.push(e);
            }
        }
    }

    /// The batch `req` failed: the same cursor can be asked for again.
    pub fn fail(&mut self, req: RequestId) {
        if self.pending == Some(req) {
            self.pending = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crustdb_core::store::ValueKind;

    use super::*;

    const WS: WorkspaceId = WorkspaceId(1);

    fn page(keys: &[&str], next: Option<&str>) -> ScanPage {
        ScanPage {
            entries: keys
                .iter()
                .map(|k| KeyEntry {
                    key: EntryKey(k.as_bytes().to_vec()),
                    kind: ValueKind::Text,
                    ttl: None,
                })
                .collect(),
            next: next.map(String::from),
        }
    }

    fn names(s: &KeyScan) -> Vec<String> {
        s.entries
            .iter()
            .map(|e| e.key.display().into_owned())
            .collect()
    }

    #[test]
    fn duplicate_scan_entries_deduped() {
        // Arrange
        let mut s = KeyScan::new(None);
        s.next(WS, RequestId(1));
        s.apply(RequestId(1), page(&["a", "b", "a"], Some("7")));
        s.next(WS, RequestId(2));

        // Act
        s.apply(RequestId(2), page(&["b", "c"], None));

        // Assert
        assert_eq!(names(&s), ["a", "b", "c"]);
    }

    #[test]
    fn scan_stops_when_cursor_exhausted() {
        // Arrange
        let mut s = KeyScan::new(Some("user:*".into()));
        let first = s.next(WS, RequestId(1));
        let busy = s.next(WS, RequestId(2));
        s.apply(RequestId(1), page(&["a"], Some("7")));

        // Act
        let second = s.next(WS, RequestId(3));
        s.apply(RequestId(3), page(&["b"], None));
        let after_end = s.next(WS, RequestId(4));

        // Assert
        assert!(matches!(
            first,
            Some(Command::ScanKeys { cursor: None, pattern: Some(p), .. }) if p == "user:*"
        ));
        assert!(busy.is_none(), "one batch in flight at a time");
        assert!(matches!(
            second,
            Some(Command::ScanKeys { cursor: Some(c), .. }) if c == "7"
        ));
        assert!(after_end.is_none() && s.is_done());
    }

    #[test]
    fn failed_batch_can_be_asked_again_from_same_cursor() {
        // Arrange
        let mut s = KeyScan::new(None);
        s.next(WS, RequestId(1));
        s.apply(RequestId(1), page(&["a"], Some("7")));
        s.next(WS, RequestId(2));

        // Act
        s.fail(RequestId(2));
        let retry = s.next(WS, RequestId(3));

        // Assert
        assert!(matches!(
            retry,
            Some(Command::ScanKeys { cursor: Some(c), .. }) if c == "7"
        ));
    }
}
