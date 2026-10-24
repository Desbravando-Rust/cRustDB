use crustdb_core::model::TypeCategory;

/// Built-in `pg_type` OIDs.
pub(crate) fn category_for_oid(oid: u32) -> TypeCategory {
    match oid {
        20 | 21 | 23 | 26 => TypeCategory::Integer,
        1700 => TypeCategory::Decimal,
        700 | 701 => TypeCategory::Float,
        18 | 19 | 25 | 1042 | 1043 => TypeCategory::Text,
        16 => TypeCategory::Bool,
        1082 => TypeCategory::Date,
        1083 | 1266 => TypeCategory::Time,
        1114 | 1184 => TypeCategory::Timestamp,
        114 | 3802 => TypeCategory::Json,
        17 => TypeCategory::Binary,
        2950 => TypeCategory::Uuid,
        _ => TypeCategory::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_builtin_oids() {
        // Arrange
        use TypeCategory::*;
        let cases = [
            (21, Integer),
            (23, Integer),
            (20, Integer),
            (26, Integer),
            (1700, Decimal),
            (700, Float),
            (701, Float),
            (25, Text),
            (1043, Text),
            (1042, Text),
            (19, Text),
            (18, Text),
            (16, Bool),
            (1082, Date),
            (1083, Time),
            (1266, Time),
            (1114, Timestamp),
            (1184, Timestamp),
            (114, Json),
            (3802, Json),
            (17, Binary),
            (2950, Uuid),
            (600, Other),
            (1007, Other),
        ];

        // Act & Assert
        for (oid, want) in cases {
            assert_eq!(category_for_oid(oid), want, "oid {oid}");
        }
    }
}
