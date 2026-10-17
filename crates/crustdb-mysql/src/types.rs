use crustdb_core::model::{Cell, TypeCategory};
use mysql_async::Column;
use mysql_async::Value;
use mysql_async::consts::{ColumnFlags, ColumnType as T};

const BINARY_CHARSET: u16 = 63;

/// Column facts needed to turn a wire value into a `Cell`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ColInfo {
    pub category: TypeCategory,
    pub decimals: u8,
    pub binary: bool,
}

impl ColInfo {
    pub(crate) fn of(c: &Column) -> Self {
        Self {
            category: category_for_column(c),
            decimals: c.decimals(),
            binary: is_binary(c),
        }
    }
}

/// Numeric and JSON columns also report charset 63, so the type decides first.
fn is_binary(c: &Column) -> bool {
    match c.column_type() {
        T::MYSQL_TYPE_BIT | T::MYSQL_TYPE_GEOMETRY => true,
        T::MYSQL_TYPE_TINY_BLOB
        | T::MYSQL_TYPE_MEDIUM_BLOB
        | T::MYSQL_TYPE_LONG_BLOB
        | T::MYSQL_TYPE_BLOB
        | T::MYSQL_TYPE_VARCHAR
        | T::MYSQL_TYPE_VAR_STRING
        | T::MYSQL_TYPE_STRING => c.character_set() == BINARY_CHARSET,
        _ => false,
    }
}

pub(crate) fn category_for_column(c: &Column) -> TypeCategory {
    use TypeCategory::*;
    match c.column_type() {
        T::MYSQL_TYPE_TINY
        | T::MYSQL_TYPE_SHORT
        | T::MYSQL_TYPE_LONG
        | T::MYSQL_TYPE_LONGLONG
        | T::MYSQL_TYPE_INT24
        | T::MYSQL_TYPE_YEAR => Integer,
        T::MYSQL_TYPE_DECIMAL | T::MYSQL_TYPE_NEWDECIMAL => Decimal,
        T::MYSQL_TYPE_FLOAT | T::MYSQL_TYPE_DOUBLE => Float,
        T::MYSQL_TYPE_DATE | T::MYSQL_TYPE_NEWDATE => Date,
        T::MYSQL_TYPE_TIME | T::MYSQL_TYPE_TIME2 => Time,
        T::MYSQL_TYPE_TIMESTAMP
        | T::MYSQL_TYPE_DATETIME
        | T::MYSQL_TYPE_TIMESTAMP2
        | T::MYSQL_TYPE_DATETIME2 => Timestamp,
        T::MYSQL_TYPE_JSON => Json,
        T::MYSQL_TYPE_BIT | T::MYSQL_TYPE_GEOMETRY | T::MYSQL_TYPE_NULL => Other,
        _ if is_binary(c) => Binary,
        _ => Text,
    }
}

pub(crate) fn type_name(c: &Column) -> String {
    let t = format!("{:?}", c.column_type());
    t.strip_prefix("MYSQL_TYPE_").unwrap_or(&t).to_lowercase()
}

pub(crate) fn nullable(c: &Column) -> Option<bool> {
    Some(!c.flags().contains(ColumnFlags::NOT_NULL_FLAG))
}

/// `.ffffff` suffix: as many digits as the column declares, or all six when the
/// column has no fixed precision (expressions) and the fraction is non-zero.
fn frac(micros: u32, decimals: u8) -> String {
    match decimals {
        1..=6 => format!(".{:06}", micros)[..=usize::from(decimals)].to_string(),
        _ if micros != 0 => format!(".{micros:06}"),
        _ => String::new(),
    }
}

/// Text-protocol values are `Bytes`; binary-protocol values (prepared statements) are
/// formatted here per §4.4.
pub(crate) fn value_to_cell(v: Value, col: &ColInfo) -> Cell {
    match v {
        Value::NULL => Cell::Null,
        Value::Bytes(b) if col.binary => Cell::bytes_capped(&b),
        Value::Bytes(b) => Cell::text_lossy_capped(&b),
        Value::Int(i) => Cell::text_capped(&i.to_string()),
        Value::UInt(i) => Cell::text_capped(&i.to_string()),
        Value::Float(f) => Cell::text_capped(&f.to_string()),
        Value::Double(f) => Cell::text_capped(&f.to_string()),
        Value::Date(y, mo, d, h, mi, s, us) => {
            let date = format!("{y:04}-{mo:02}-{d:02}");
            // DATE columns carry no time part even though the wire value does.
            if col.category == TypeCategory::Date {
                Cell::text_capped(&date)
            } else {
                Cell::text_capped(&format!(
                    "{date} {h:02}:{mi:02}:{s:02}{}",
                    frac(us, col.decimals)
                ))
            }
        }
        Value::Time(neg, days, h, mi, s, us) => {
            let sign = if neg { "-" } else { "" };
            let hours = u32::from(h).saturating_add(days.saturating_mul(24));
            Cell::text_capped(&format!(
                "{sign}{hours:02}:{mi:02}:{s:02}{}",
                frac(us, col.decimals)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(t: T, charset: u16) -> Column {
        Column::new(t).with_character_set(charset)
    }
    fn txt(c: Cell) -> String {
        match c {
            Cell::Text { value, .. } => value.into(),
            o => panic!("{o:?}"),
        }
    }
    fn info(t: T, decimals: u8) -> ColInfo {
        ColInfo::of(&Column::new(t).with_decimals(decimals))
    }

    #[test]
    fn categories() {
        use TypeCategory::*;
        let cases = [
            (T::MYSQL_TYPE_LONG, 63, Integer),
            (T::MYSQL_TYPE_NEWDECIMAL, 63, Decimal),
            (T::MYSQL_TYPE_DOUBLE, 63, Float),
            (T::MYSQL_TYPE_JSON, 63, Json),
            (T::MYSQL_TYPE_DATE, 63, Date),
            (T::MYSQL_TYPE_TIME, 63, Time),
            (T::MYSQL_TYPE_DATETIME, 63, Timestamp),
            (T::MYSQL_TYPE_TIMESTAMP, 63, Timestamp),
            (T::MYSQL_TYPE_BLOB, 63, Binary),
            (T::MYSQL_TYPE_BLOB, 45, Text),
            (T::MYSQL_TYPE_VAR_STRING, 63, Binary),
            (T::MYSQL_TYPE_VAR_STRING, 8, Text),
            (T::MYSQL_TYPE_STRING, 63, Binary),
            (T::MYSQL_TYPE_BIT, 63, Other),
        ];
        for (t, cs, want) in cases {
            assert_eq!(category_for_column(&col(t, cs)), want, "{t:?}/{cs}");
        }
    }

    #[test]
    fn text_protocol_bytes_follow_charset() {
        let bin = ColInfo::of(&col(T::MYSQL_TYPE_BLOB, 63));
        let text = ColInfo::of(&col(T::MYSQL_TYPE_VAR_STRING, 45));
        let json = ColInfo::of(&col(T::MYSQL_TYPE_JSON, 63));
        assert!(matches!(
            value_to_cell(Value::Bytes(vec![0, 1]), &bin),
            Cell::Bytes { total_len: 2, .. }
        ));
        assert_eq!(txt(value_to_cell(Value::Bytes("é".into()), &text)), "é");
        assert_eq!(
            txt(value_to_cell(Value::Bytes(b"{}".to_vec()), &json)),
            "{}"
        );
        assert_eq!(value_to_cell(Value::NULL, &text), Cell::Null);
    }

    #[test]
    fn binary_protocol_dates_and_times() {
        let date = info(T::MYSQL_TYPE_DATE, 0);
        let dt0 = info(T::MYSQL_TYPE_DATETIME, 0);
        let dt6 = info(T::MYSQL_TYPE_DATETIME, 6);
        let dt3 = info(T::MYSQL_TYPE_DATETIME, 3);
        let expr = info(T::MYSQL_TYPE_DATETIME, 31);
        let time = info(T::MYSQL_TYPE_TIME, 0);
        let time6 = info(T::MYSQL_TYPE_TIME, 6);
        let d = |us| Value::Date(2024, 1, 2, 3, 4, 5, us);
        assert_eq!(
            txt(value_to_cell(Value::Date(2024, 1, 2, 0, 0, 0, 0), &date)),
            "2024-01-02"
        );
        assert_eq!(txt(value_to_cell(d(0), &dt0)), "2024-01-02 03:04:05");
        assert_eq!(txt(value_to_cell(d(0), &dt6)), "2024-01-02 03:04:05.000000");
        assert_eq!(
            txt(value_to_cell(d(120_000), &dt3)),
            "2024-01-02 03:04:05.120"
        );
        assert_eq!(
            txt(value_to_cell(d(7), &expr)),
            "2024-01-02 03:04:05.000007"
        );
        assert_eq!(txt(value_to_cell(d(0), &expr)), "2024-01-02 03:04:05");
        assert_eq!(
            txt(value_to_cell(Value::Time(true, 34, 22, 59, 59, 0), &time)),
            "-838:59:59"
        );
        assert_eq!(
            txt(value_to_cell(
                Value::Time(false, 0, 1, 2, 3, 500_000),
                &time6
            )),
            "01:02:03.500000"
        );
    }

    #[test]
    fn numbers_are_decimal_text() {
        let c = info(T::MYSQL_TYPE_LONGLONG, 0);
        assert_eq!(txt(value_to_cell(Value::Int(-5), &c)), "-5");
        assert_eq!(
            txt(value_to_cell(Value::UInt(u64::MAX), &c)),
            "18446744073709551615"
        );
        assert_eq!(txt(value_to_cell(Value::Double(1.5), &c)), "1.5");
        assert_eq!(txt(value_to_cell(Value::Float(2.5), &c)), "2.5");
    }
}
