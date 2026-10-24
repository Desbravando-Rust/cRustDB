use crustdb_core::model::{Cell, TypeCategory};
use rusqlite::types::ValueRef;

/// Category of a declared column type, by SQLite's affinity rules (datatype3 §3.1).
/// Expressions (no declared type) and untyped columns are `Other`.
pub(crate) fn category(decl: Option<&str>) -> TypeCategory {
    let Some(d) = decl.map(str::to_ascii_uppercase) else {
        return TypeCategory::Other;
    };
    let has = |p: &[&str]| p.iter().any(|s| d.contains(*s));
    if d.contains("INT") {
        TypeCategory::Integer
    } else if has(&["CHAR", "CLOB", "TEXT"]) {
        TypeCategory::Text
    } else if d.contains("BLOB") {
        TypeCategory::Binary
    } else if d.trim().is_empty() {
        TypeCategory::Other
    } else if has(&["REAL", "FLOA", "DOUB"]) {
        TypeCategory::Float
    } else {
        TypeCategory::Decimal
    }
}

/// Cells follow the stored value's type, not the column's (SQLite is dynamically typed).
pub(crate) fn value_to_cell(v: ValueRef<'_>) -> Cell {
    match v {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(i) => Cell::text_capped(&i.to_string()),
        ValueRef::Real(f) => Cell::text_capped(&f.to_string()),
        ValueRef::Text(b) => Cell::text_lossy_capped(b),
        ValueRef::Blob(b) => Cell::bytes_capped(b),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn categories_follow_affinity() {
        // Arrange
        use TypeCategory::*;
        let cases = [
            (Some("INTEGER"), Integer),
            (Some("bigint"), Integer),
            (Some("VARCHAR(10)"), Text),
            (Some("text"), Text),
            (Some("BLOB"), Binary),
            (Some(""), Other),
            (None, Other),
            (Some("DOUBLE PRECISION"), Float),
            (Some("NUMERIC(10,2)"), Decimal),
            (Some("DATETIME"), Decimal),
        ];

        // Act & Assert
        for (decl, want) in cases {
            assert_eq!(category(decl), want, "{decl:?}");
        }
    }

    #[test]
    fn cells_follow_value_type() {
        // Act & Assert
        assert_eq!(value_to_cell(ValueRef::Null), Cell::Null);
        assert_eq!(
            value_to_cell(ValueRef::Integer(-5)),
            Cell::text_capped("-5")
        );
        assert_eq!(value_to_cell(ValueRef::Real(1.5)), Cell::text_capped("1.5"));
        assert_eq!(
            value_to_cell(ValueRef::Text("é".as_bytes())),
            Cell::text_capped("é")
        );
        assert!(matches!(
            value_to_cell(ValueRef::Blob(&[0, 1])),
            Cell::Bytes { total_len: 2, .. }
        ));
    }
}
