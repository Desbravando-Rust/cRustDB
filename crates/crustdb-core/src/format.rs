use std::borrow::Cow;
use std::fmt::Write;

use crate::model::{Cell, ColumnMeta};

const BINARY_PREVIEW_BYTES: usize = 16;

pub fn display_cell<'a>(cell: &'a Cell, _col: &ColumnMeta, max_chars: usize) -> Cow<'a, str> {
    match cell {
        Cell::Null => Cow::Borrowed("NULL"),
        Cell::Text { value, truncated } => match value.char_indices().nth(max_chars) {
            Some((end, _)) => Cow::Owned(format!("{}…", &value[..end])),
            None if *truncated => Cow::Owned(format!("{value}…")),
            None => Cow::Borrowed(value),
        },
        Cell::Bytes {
            value, total_len, ..
        } => Cow::Owned(format!(
            "(binary {}) {}",
            human_size(*total_len),
            hex_preview(value, BINARY_PREVIEW_BYTES)
        )),
    }
}

pub fn pretty_json(s: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    serde_json::to_string_pretty(&v).ok()
}

pub fn hex_preview(b: &[u8], n: usize) -> String {
    b.iter().take(n).fold(String::new(), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    })
}

/// Classic 16-bytes-per-line dump (offset, hex, ASCII) of at most `max` bytes.
pub fn hex_dump(b: &[u8], max: usize) -> String {
    let shown = &b[..b.len().min(max)];
    let mut out = String::new();
    for (i, chunk) in shown.chunks(16).enumerate() {
        let _ = write!(out, "{:08x}  ", i * 16);
        for j in 0..16 {
            match chunk.get(j) {
                Some(x) => {
                    let _ = write!(out, "{x:02x} ");
                }
                None => out.push_str("   "),
            }
        }
        out.push(' ');
        out.extend(chunk.iter().map(|&x| {
            if (0x20..0x7f).contains(&x) {
                x as char
            } else {
                '.'
            }
        }));
        out.push('\n');
    }
    if shown.len() < b.len() {
        let _ = writeln!(out, "… {} more bytes", b.len() - shown.len());
    }
    out
}

/// Base 1000, one decimal (truncated, not rounded).
pub fn human_size(n: u64) -> String {
    if n < 1000 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut unit = "B";
    for u in ["KB", "MB", "GB"] {
        v /= 1000.0;
        unit = u;
        if v < 1000.0 {
            break;
        }
    }
    format!("{:.1} {unit}", (v * 10.0).floor() / 10.0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::model::{CELL_MEMORY_CAP, TypeCategory};

    fn col(category: TypeCategory) -> ColumnMeta {
        ColumnMeta {
            name: "c".into(),
            type_name: "t".into(),
            category,
            nullable: None,
            is_primary_key: false,
        }
    }

    #[test]
    fn null_displays_as_null() {
        // Act & Assert
        assert_eq!(
            display_cell(&Cell::Null, &col(TypeCategory::Text), 256),
            "NULL"
        );
    }

    #[test]
    fn text_truncated_at_max_chars() {
        // Arrange
        let c = Cell::text_capped(&"é".repeat(300));

        // Act & Assert
        assert_eq!(
            display_cell(&c, &col(TypeCategory::Text), 256)
                .chars()
                .count(),
            257
        );
    }

    #[test]
    fn memory_truncated_text_gets_ellipsis() {
        // Arrange
        let c = Cell::text_capped(&"a".repeat(CELL_MEMORY_CAP + 10));

        // Act & Assert
        assert!(display_cell(&c, &col(TypeCategory::Text), usize::MAX).ends_with('…'));
    }

    #[test]
    fn binary_shows_size_and_preview() {
        // Arrange
        let c = Cell::bytes_capped(&[0xde, 0xad, 0xbe, 0xef]);

        // Act & Assert
        assert_eq!(
            display_cell(&c, &col(TypeCategory::Binary), 256),
            "(binary 4 B) deadbeef"
        );
    }

    #[test]
    fn cell_memory_capped_at_1mib() {
        // Act
        let c = Cell::text_capped(&"a".repeat(CELL_MEMORY_CAP + 10));

        // Assert
        assert!(
            matches!(c, Cell::Text { truncated: true, ref value } if value.len() == CELL_MEMORY_CAP)
        );
    }

    #[test]
    fn invalid_utf8_text_displays_lossy() {
        // Arrange
        let c = Cell::text_lossy_capped(&[b'a', 0xff, b'b']);

        // Act & Assert
        assert_eq!(
            display_cell(&c, &col(TypeCategory::Text), 256),
            "a\u{FFFD}b"
        );
    }

    #[test]
    fn human_size_formats() {
        // Act & Assert
        assert_eq!(human_size(2_355), "2.3 KB");
        assert_eq!(human_size(4), "4 B");
    }

    #[test]
    fn pretty_json_formats_objects() {
        // Act & Assert
        assert_eq!(
            pretty_json(r#"{"k":1}"#).as_deref(),
            Some("{\n  \"k\": 1\n}")
        );
    }

    #[test]
    fn hex_dump_formats_line() {
        // Act & Assert
        assert!(hex_dump(b"AB", 16).starts_with("00000000  41 42 "));
    }
}
