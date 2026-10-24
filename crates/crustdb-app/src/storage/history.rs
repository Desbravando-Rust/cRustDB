use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crustdb_core::model::ConnectionId;
use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub ts: String,
    pub database: Option<String>,
    pub sql: String,
    pub duration_ms: u64,
    pub rows: Option<u64>,
    pub ok: bool,
    pub error: Option<String>,
}

pub struct HistoryStore {
    pub dir: PathBuf,
}

/// Current time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_iso8601(secs)
}

pub fn format_iso8601(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant), epoch shifted to 0000-03-01.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

impl HistoryStore {
    fn path(&self, id: ConnectionId) -> PathBuf {
        self.dir.join(format!("{}.jsonl", id.0))
    }

    pub fn append(&self, id: ConnectionId, e: &HistoryEntry) -> io::Result<()> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;
        let mut line = serde_json::to_vec(e).map_err(io::Error::other)?;
        line.push(b'\n');
        OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(self.path(id))?
            .write_all(&line)
    }

    pub fn load(&self, id: ConnectionId) -> io::Result<Vec<HistoryEntry>> {
        let path = self.path(id);
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut lines = 0usize;
        let mut entries = Vec::new();
        for l in BufReader::new(file).split(b'\n') {
            lines += 1;
            // IO errors propagate; only unparsable lines count as corrupt.
            if let Ok(e) = serde_json::from_slice(&l?) {
                entries.push(e);
            }
        }
        if lines > MAX_ENTRIES {
            entries.drain(..entries.len().saturating_sub(MAX_ENTRIES));
            let mut tmp = path.clone().into_os_string();
            tmp.push(".tmp");
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            for e in &entries {
                serde_json::to_writer(&mut f, e).map_err(io::Error::other)?;
                f.write_all(b"\n")?;
            }
            f.sync_all()?;
            fs::rename(&tmp, &path)?;
        }
        Ok(entries)
    }

    pub fn search<'a>(entries: &'a [HistoryEntry], q: &str) -> Vec<&'a HistoryEntry> {
        let q = q.to_lowercase();
        entries
            .iter()
            .filter(|e| e.sql.to_lowercase().contains(&q))
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn entry(sql: &str) -> HistoryEntry {
        HistoryEntry {
            ts: format_iso8601(0),
            database: None,
            sql: sql.into(),
            duration_ms: 1,
            rows: Some(2),
            ok: true,
            error: None,
        }
    }

    fn store() -> (tempfile::TempDir, HistoryStore, ConnectionId) {
        let t = tempfile::tempdir().unwrap();
        let s = HistoryStore {
            dir: t.path().join("history"),
        };
        (t, s, ConnectionId(uuid::Uuid::from_u128(1)))
    }

    #[test]
    fn iso8601_formats_known_instants() {
        // Act & Assert
        assert_eq!(format_iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_iso8601(1_792_245_791), "2026-10-17T14:03:11Z");
        assert_eq!(format_iso8601(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn history_appends_jsonl_with_0600() {
        // Arrange
        let (_t, s, id) = store();
        s.append(id, &entry("select 1")).unwrap();

        // Act
        s.append(id, &entry("select 2")).unwrap();
        let p = s.path(id);

        // Assert
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_to_string(&p).unwrap().lines().count(), 2);
        assert_eq!(s.load(id).unwrap()[1].sql, "select 2");
    }

    #[test]
    fn history_compacts_to_limit() {
        // Arrange
        let (_t, s, id) = store();
        for i in 0..10_050 {
            s.append(id, &entry(&format!("q{i}"))).unwrap();
        }

        // Act
        let e = s.load(id).unwrap();

        // Assert
        assert_eq!(e.len(), 10_000);
        assert_eq!(e[0].sql, "q50");
        assert_eq!(e[9_999].sql, "q10049");
        assert_eq!(
            fs::read_to_string(s.path(id)).unwrap().lines().count(),
            10_000
        );
        assert_eq!(
            fs::metadata(s.path(id)).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn history_search_substring_case_insensitive() {
        // Arrange
        let v = vec![entry("SELECT * FROM Users"), entry("delete from t")];

        // Act
        let r = HistoryStore::search(&v, "users");

        // Assert
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].sql, "SELECT * FROM Users");
        assert_eq!(HistoryStore::search(&v, "").len(), 2);
    }

    #[test]
    fn history_skips_corrupt_lines() {
        // Arrange
        let (_t, s, id) = store();
        s.append(id, &entry("a")).unwrap();
        let mut f = OpenOptions::new().append(true).open(s.path(id)).unwrap();
        f.write_all(b"{not json\n\xff\xfe\n").unwrap();
        s.append(id, &entry("b")).unwrap();

        // Act
        let e = s.load(id).unwrap();

        // Assert
        assert_eq!(
            e.iter().map(|e| e.sql.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
    }

    #[test]
    fn history_load_propagates_io_errors() {
        // Arrange
        let (_t, s, id) = store();
        fs::create_dir_all(s.path(id)).unwrap();

        // Act & Assert
        assert!(s.load(id).is_err());
    }
}
