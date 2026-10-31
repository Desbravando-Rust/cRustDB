use super::Reply;
use crate::model::{Cell, ColumnMeta, Row, TypeCategory};

#[derive(Debug, Clone, PartialEq)]
pub struct StreamEntry {
    pub id: String,
    pub fields: Vec<(Vec<u8>, Vec<u8>)>,
}

/// Decoded value of one key, by structure.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueBody {
    Text(Vec<u8>),
    Map(Vec<(Vec<u8>, Vec<u8>)>),
    List(Vec<Vec<u8>>),
    Set(Vec<Vec<u8>>),
    SortedSet(Vec<(Vec<u8>, f64)>),
    Stream(Vec<StreamEntry>),
}

impl Reply {
    /// Grid view of a command reply: arrays one row per element, maps `key`/`value`, scalars one row.
    pub fn to_rows(&self) -> (Vec<ColumnMeta>, Vec<Row>) {
        let value = || col("value", TypeCategory::Text);
        match self {
            Reply::Array(items) => (
                vec![value()],
                items.iter().map(|r| Row::from([reply_cell(r)])).collect(),
            ),
            Reply::Map(pairs) => (
                vec![col("key", TypeCategory::Text), value()],
                pairs
                    .iter()
                    .map(|(k, v)| Row::from([reply_cell(k), reply_cell(v)]))
                    .collect(),
            ),
            one => (vec![value()], vec![Row::from([reply_cell(one)])]),
        }
    }
}

fn reply_cell(r: &Reply) -> Cell {
    match r {
        Reply::Nil => Cell::Null,
        Reply::Bulk(b) => Cell::text_capped(&String::from_utf8_lossy(b)),
        other => text(reply_text(other)),
    }
}

fn reply_text(r: &Reply) -> String {
    match r {
        Reply::Nil => "(nil)".into(),
        Reply::Int(n) => n.to_string(),
        Reply::Double(d) => d.to_string(),
        Reply::Bool(b) => b.to_string(),
        Reply::Bulk(b) => String::from_utf8_lossy(b).into_owned(),
        Reply::Status(s) => s.clone(),
        Reply::Array(a) => format!(
            "[{}]",
            a.iter().map(reply_text).collect::<Vec<_>>().join(", ")
        ),
        Reply::Map(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, v)| format!("{}: {}", reply_text(k), reply_text(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn col(name: &str, category: TypeCategory) -> ColumnMeta {
    ColumnMeta {
        name: name.to_owned(),
        type_name: match category {
            TypeCategory::Integer => "integer",
            TypeCategory::Float => "float",
            _ => "text",
        }
        .to_owned(),
        category,
        nullable: Some(false),
        is_primary_key: false,
    }
}

fn cell(bytes: &[u8]) -> Cell {
    match std::str::from_utf8(bytes) {
        Ok(s) => Cell::text_capped(s),
        Err(_) => Cell::bytes_capped(bytes),
    }
}

fn text(s: String) -> Cell {
    Cell::text_capped(&s)
}

impl ValueBody {
    /// Grid view of the value; order is the store's order.
    pub fn to_rows(&self) -> (Vec<ColumnMeta>, Vec<Row>) {
        let value = || col("value", TypeCategory::Text);
        match self {
            ValueBody::Text(v) => (vec![value()], vec![Row::from([cell(v)])]),
            ValueBody::Map(m) => (
                vec![col("field", TypeCategory::Text), value()],
                m.iter()
                    .map(|(f, v)| Row::from([cell(f), cell(v)]))
                    .collect(),
            ),
            ValueBody::List(l) => (
                vec![col("index", TypeCategory::Integer), value()],
                l.iter()
                    .enumerate()
                    .map(|(i, v)| Row::from([text(i.to_string()), cell(v)]))
                    .collect(),
            ),
            ValueBody::Set(s) => (
                vec![col("member", TypeCategory::Text)],
                s.iter().map(|m| Row::from([cell(m)])).collect(),
            ),
            ValueBody::SortedSet(z) => (
                vec![
                    col("member", TypeCategory::Text),
                    col("score", TypeCategory::Float),
                ],
                z.iter()
                    .map(|(m, sc)| Row::from([cell(m), text(sc.to_string())]))
                    .collect(),
            ),
            ValueBody::Stream(entries) => (
                vec![
                    col("id", TypeCategory::Text),
                    col("fields", TypeCategory::Text),
                ],
                entries
                    .iter()
                    .map(|e| Row::from([text(e.id.clone()), text(join_fields(&e.fields))]))
                    .collect(),
            ),
        }
    }
}

// ponytail: lossy display only (non-UTF-8 -> U+FFFD); joined string is built before the cell cap, bounded by the read limit
fn join_fields(fields: &[(Vec<u8>, Vec<u8>)]) -> String {
    let parts: Vec<String> = fields
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                String::from_utf8_lossy(k),
                String::from_utf8_lossy(v)
            )
        })
        .collect();
    parts.join(", ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn t(s: &str) -> Cell {
        Cell::text_capped(s)
    }

    fn names(cols: &[ColumnMeta]) -> Vec<&str> {
        cols.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn hash_body_maps_to_field_value_rows() {
        // Arrange
        let body = ValueBody::Map(vec![(b"name".to_vec(), b"ana".to_vec())]);

        // Act
        let (cols, rows) = body.to_rows();

        // Assert
        assert_eq!(names(&cols), ["field", "value"]);
        assert_eq!(rows, vec![Row::from([t("name"), t("ana")])]);
    }

    #[test]
    fn zset_rows_keep_score_order() {
        // Arrange
        let body = ValueBody::SortedSet(vec![(b"b".to_vec(), 1.0), (b"a".to_vec(), 2.0)]);

        // Act
        let (cols, rows) = body.to_rows();

        // Assert
        assert_eq!(names(&cols), ["member", "score"]);
        assert_eq!(cols[1].type_name, "float");
        assert_eq!(rows[0][0], t("b"));
        assert_eq!(rows[0][1], t("1"));
        assert_eq!(rows[1][0], t("a"));
        assert_eq!(rows[1][1], t("2"));
    }

    #[test]
    fn stream_rows_join_fields_per_id() {
        // Arrange
        let body = ValueBody::Stream(vec![StreamEntry {
            id: "1-0".into(),
            fields: vec![
                (b"k".to_vec(), b"v".to_vec()),
                (b"k2".to_vec(), b"v2".to_vec()),
            ],
        }]);

        // Act
        let (cols, rows) = body.to_rows();

        // Assert
        assert_eq!(names(&cols), ["id", "fields"]);
        assert_eq!(rows, vec![Row::from([t("1-0"), t("k=v, k2=v2")])]);
    }

    #[test]
    fn binary_value_maps_to_bytes_cell() {
        // Arrange
        let body = ValueBody::Text(vec![0xff, 0x00]);

        // Act
        let (_, rows) = body.to_rows();

        // Assert
        assert_eq!(rows[0][0], Cell::bytes_capped(&[0xff, 0x00]));
    }

    #[test]
    fn list_rows_carry_position() {
        // Arrange
        let body = ValueBody::List(vec![b"x".to_vec(), b"y".to_vec()]);

        // Act
        let (cols, rows) = body.to_rows();

        // Assert
        assert_eq!(names(&cols), ["index", "value"]);
        assert_eq!(cols[0].type_name, "integer");
        assert_eq!(cols[1].type_name, "text");
        assert_eq!(rows[1][0], t("1"));
        assert_eq!(rows[1][1], t("y"));
    }

    #[test]
    fn array_reply_maps_to_one_row_per_element() {
        // Arrange
        let reply = Reply::Array(vec![Reply::Bulk(b"a".to_vec()), Reply::Int(2), Reply::Nil]);

        // Act
        let (cols, rows) = reply.to_rows();

        // Assert
        assert_eq!(names(&cols), ["value"]);
        assert_eq!(
            rows,
            vec![
                Row::from([t("a")]),
                Row::from([t("2")]),
                Row::from([Cell::Null])
            ]
        );
    }

    #[test]
    fn map_and_scalar_replies_map_to_rows() {
        // Arrange
        let map = Reply::Map(vec![(Reply::Bulk(b"k".to_vec()), Reply::Int(1))]);

        // Act
        let (cols, rows) = map.to_rows();
        let (_, one) = Reply::Status("OK".into()).to_rows();

        // Assert
        assert_eq!(names(&cols), ["key", "value"]);
        assert_eq!(rows, vec![Row::from([t("k"), t("1")])]);
        assert_eq!(one, vec![Row::from([t("OK")])]);
    }

    #[test]
    fn nested_reply_text_is_capped() {
        // Arrange
        let big = Reply::Array(vec![Reply::Bulk(vec![
            b'x';
            crate::model::CELL_MEMORY_CAP * 2
        ])]);

        // Act
        let (_, rows) = Reply::Array(vec![big]).to_rows();

        // Assert
        assert!(matches!(
            &rows[0][0],
            Cell::Text {
                truncated: true,
                ..
            }
        ));
    }
}
