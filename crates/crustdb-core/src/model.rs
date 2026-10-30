use serde::{Deserialize, Serialize};

pub const CELL_MEMORY_CAP: usize = 1_048_576;
pub const GRID_DISPLAY_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Postgres,
    Mysql,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Product {
    Postgres,
    Mysql,
    Mariadb,
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionId(pub uuid::Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectKind {
    Database,
    Schema,
    Table,
    View,
    MaterializedView,
    Function,
    Procedure,
    Trigger,
    Sequence,
    Type,
}

/// MySQL objects have `schema = None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub database: String,
    pub schema: Option<String>,
    pub name: String,
    pub kind: ObjectKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeCategory {
    Integer,
    Decimal,
    Float,
    Text,
    Bool,
    Date,
    Time,
    Timestamp,
    Json,
    Binary,
    Uuid,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
    pub category: TypeCategory,
    pub nullable: Option<bool>,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDetail {
    pub meta: ColumnMeta,
    pub default: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: ObjectRef,
    pub ref_columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Null,
    Text {
        value: Box<str>,
        truncated: bool,
    },
    Bytes {
        value: Box<[u8]>,
        total_len: u64,
        truncated: bool,
    },
}

impl Cell {
    pub fn text_capped(s: &str) -> Cell {
        if s.len() <= CELL_MEMORY_CAP {
            return Cell::Text {
                value: s.into(),
                truncated: false,
            };
        }
        let end = (0..=CELL_MEMORY_CAP)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        Cell::Text {
            value: s[..end].into(),
            truncated: true,
        }
    }

    pub fn text_lossy_capped(b: &[u8]) -> Cell {
        Cell::text_capped(&String::from_utf8_lossy(b))
    }

    pub fn bytes_capped(b: &[u8]) -> Cell {
        let keep = b.len().min(CELL_MEMORY_CAP);
        Cell::Bytes {
            value: b[..keep].into(),
            total_len: b.len() as u64,
            truncated: keep < b.len(),
        }
    }
}

pub type Row = Box<[Cell]>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerVersion {
    pub product: Product,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub affected_rows: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataModel {
    Relational,
    KeyValue,
}

/// Which UI surfaces a data model supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub table_tab: bool,
    pub er_diagram: bool,
    pub table_export_csv: bool,
    pub sql_editor: bool,
    pub row_edit: bool,
    pub ai_query: bool,
    pub command_editor: bool,
}

impl DataModel {
    pub const fn capabilities(self) -> Capabilities {
        match self {
            DataModel::Relational => Capabilities {
                table_tab: true,
                er_diagram: true,
                table_export_csv: true,
                sql_editor: true,
                row_edit: true,
                ai_query: true,
                command_editor: false,
            },
            DataModel::KeyValue => Capabilities {
                table_tab: false,
                er_diagram: false,
                table_export_csv: false,
                sql_editor: false,
                row_edit: false,
                ai_query: false,
                command_editor: true,
            },
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn key_value_model_disables_sql_screens() {
        // Act
        let caps = DataModel::KeyValue.capabilities();

        // Assert
        assert_eq!(
            caps,
            Capabilities {
                table_tab: false,
                er_diagram: false,
                table_export_csv: false,
                sql_editor: false,
                row_edit: false,
                ai_query: false,
                command_editor: true,
            }
        );
    }

    #[test]
    fn relational_model_keeps_sql_screens() {
        // Act
        let caps = DataModel::Relational.capabilities();

        // Assert
        assert_eq!(
            caps,
            Capabilities {
                table_tab: true,
                er_diagram: true,
                table_export_csv: true,
                sql_editor: true,
                row_edit: true,
                ai_query: true,
                command_editor: false,
            }
        );
    }
}
