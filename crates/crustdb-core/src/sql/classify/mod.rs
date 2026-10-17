use std::ops::ControlFlow;

use sqlparser::ast::{
    CopyTarget, FromTable, ObjectName, ObjectType, SetExpr, Statement, TableFactor, TableObject,
    TableWithJoins, visit_statements,
};
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer};

use crate::driver::ExecMode;
use crate::model::DriverKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxOp {
    Begin,
    Commit,
    Rollback,
    Savepoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmlKind {
    Insert,
    Update,
    Delete,
    Merge,
    Replace,
    LoadData,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestructiveKind {
    DropTable,
    DropOther,
    Truncate,
    Alter,
    Rename,
    CreateOrReplace,
    Grant,
    Revoke,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatementClass {
    Read,
    Session,
    Tx(TxOp),
    CreateSafe,
    Dml(DmlKind),
    Destructive(DestructiveKind),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetName {
    pub qualifier: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub class: StatementClass,
    pub has_where: bool,
    pub values_rows: Option<u64>,
    pub target: Option<TargetName>,
    /// Plain query that can stream through a server-side cursor.
    pub cursorable: bool,
    /// DDL (implicit commit on MySQL).
    pub is_ddl: bool,
}

impl Classified {
    fn new(class: StatementClass) -> Self {
        let is_ddl = matches!(
            class,
            StatementClass::CreateSafe | StatementClass::Destructive(_)
        );
        Self {
            class,
            has_where: false,
            values_rows: None,
            target: None,
            cursorable: false,
            is_ddl,
        }
    }

    fn target(mut self, name: Option<&ObjectName>) -> Self {
        self.target = name.and_then(target_of);
        self
    }

    pub fn exec_mode(&self) -> ExecMode {
        if self.cursorable {
            ExecMode::Read
        } else {
            ExecMode::Other
        }
    }
}

// Both driver lists are matched for either driver (the names do not collide in practice).
const SIDE_EFFECT_FNS: &[&str] = &[
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "set_config",
    "setval",
    "nextval",
    "lo_unlink",
    "dblink_exec",
    "get_lock",
    "release_lock",
    "release_all_locks",
];

fn target_of(name: &ObjectName) -> Option<TargetName> {
    let mut parts = name.0.iter().filter_map(|p| p.as_ident()).rev();
    let last = parts.next()?.value.clone();
    Some(TargetName {
        qualifier: parts.next().map(|i| i.value.clone()),
        name: last,
    })
}

/// Token scan (not the AST) so table-position calls like `FROM pg_terminate_backend(1)` are caught.
fn has_side_effect_fn(toks: &[Token]) -> bool {
    let mut it = toks
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace(_)))
        .peekable();
    while let Some(t) = it.next() {
        if let Token::Word(w) = t {
            let n = w.value.to_ascii_lowercase();
            if (SIDE_EFFECT_FNS.contains(&n.as_str())
                || n.starts_with("pg_advisory_")
                || n.starts_with("pg_try_advisory_"))
                && it.peek() == Some(&&Token::LParen)
            {
                return true;
            }
        }
    }
    false
}

/// MySQL `LOAD DATA INFILE ...` is not parsed by sqlparser; detect it from tokens.
fn load_data(toks: &[Token]) -> Option<Classified> {
    let words: Vec<&Token> = toks
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace(_)))
        .collect();
    let is = |i: usize, kw: &str| matches!(words.get(i), Some(Token::Word(w)) if w.value.eq_ignore_ascii_case(kw));
    if !(is(0, "LOAD") && is(1, "DATA")) {
        return None;
    }
    let mut c = Classified::new(StatementClass::Dml(DmlKind::LoadData));
    if let Some(i) = (2..words.len()).find(|&i| is(i, "TABLE")) {
        let word = |j: usize| match words.get(j) {
            Some(Token::Word(w)) => Some(w.value.clone()),
            _ => None,
        };
        c.target = match (word(i + 1), words.get(i + 2), word(i + 3)) {
            (Some(q), Some(Token::Period), Some(n)) => Some(TargetName {
                qualifier: Some(q),
                name: n,
            }),
            (Some(n), ..) => Some(TargetName {
                qualifier: None,
                name: n,
            }),
            _ => None,
        };
    }
    Some(c)
}

fn table_factor_name(t: &TableFactor) -> Option<&ObjectName> {
    match t {
        TableFactor::Table { name, .. } => Some(name),
        _ => None,
    }
}

fn first_table(t: &[TableWithJoins]) -> Option<&ObjectName> {
    t.first().and_then(|t| table_factor_name(&t.relation))
}

pub fn classify(sql: &str, kind: DriverKind) -> Classified {
    let dialect = super::dialect(kind);
    let unknown = Classified::new(StatementClass::Unknown);
    let toks = Tokenizer::new(dialect, sql).tokenize().unwrap_or_default();
    let Ok(mut stmts) = Parser::parse_sql(dialect, sql) else {
        return load_data(&toks)
            .or_else(|| explain_analyse(sql, kind))
            .unwrap_or(unknown);
    };
    if stmts.len() != 1 {
        return unknown;
    }
    let Some(stmt) = stmts.pop() else {
        return unknown;
    };
    let c = of_stmt(&stmt);
    if matches!(
        c.class,
        StatementClass::Read
            | StatementClass::Session
            | StatementClass::Tx(_)
            | StatementClass::CreateSafe
    ) && has_side_effect_fn(&toks)
    {
        // Keep DDL-ness: MySQL still commits implicitly once the user confirms.
        return Classified {
            is_ddl: c.is_ddl,
            target: c.target,
            ..unknown
        };
    }
    c
}

/// sqlparser rejects the PG spelling `EXPLAIN ANALYSE`; classify the inner statement instead.
fn explain_analyse(sql: &str, kind: DriverKind) -> Option<Classified> {
    let (a, rest) = sql.trim_start().split_once(char::is_whitespace)?;
    let (b, inner) = rest.trim_start().split_once(char::is_whitespace)?;
    if !(a.eq_ignore_ascii_case("EXPLAIN") && b.eq_ignore_ascii_case("ANALYSE")) {
        return None;
    }
    Some(Classified {
        cursorable: false,
        ..classify(inner, kind)
    })
}

/// Classifies `stmt`, upgrading "harmless" classes when DML is nested inside (CTE, `COPY (DELETE ...)`).
fn of_stmt(stmt: &Statement) -> Classified {
    let c = base_class(stmt);
    // Plain EXPLAIN never executes its inner statement.
    if !matches!(
        c.class,
        StatementClass::Read
            | StatementClass::Session
            | StatementClass::Tx(_)
            | StatementClass::CreateSafe
    ) || matches!(stmt, Statement::Explain { .. })
    {
        return c;
    }
    let mut nested = None;
    let _ = visit_statements(stmt, |s| {
        if matches!(
            s,
            Statement::Insert(_)
                | Statement::Update(_)
                | Statement::Delete(_)
                | Statement::Merge(_)
        ) {
            nested = Some(base_class(s));
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    nested.unwrap_or(c)
}

fn base_class(stmt: &Statement) -> Classified {
    use DestructiveKind as D;
    use StatementClass as C;
    let destructive = |k| Classified::new(C::Destructive(k));
    let dml = |k| Classified::new(C::Dml(k));
    match stmt {
        Statement::Query(q) => Classified {
            // SELECT ... INTO creates a table, so it cannot go through a cursor.
            cursorable: !matches!(q.body.as_ref(), SetExpr::Select(sel) if sel.into.is_some()),
            ..Classified::new(C::Read)
        },
        Statement::Insert(i) => {
            let name = match &i.table {
                TableObject::TableName(n) => Some(n),
                _ => None,
            };
            let values_rows = match i.source.as_deref().map(|q| q.body.as_ref()) {
                Some(SetExpr::Values(v)) => Some(v.rows.len() as u64),
                _ => None,
            };
            Classified {
                values_rows,
                ..dml(if i.replace_into {
                    DmlKind::Replace
                } else {
                    DmlKind::Insert
                })
                .target(name)
            }
        }
        // ponytail: multi-table `UPDATE a JOIN b` targets the first table only.
        Statement::Update(u) => Classified {
            has_where: u.selection.is_some(),
            ..dml(DmlKind::Update).target(table_factor_name(&u.table.relation))
        },
        Statement::Delete(d) => {
            let (FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t)) = &d.from;
            Classified {
                has_where: d.selection.is_some(),
                ..dml(DmlKind::Delete).target(d.tables.first().or(first_table(t)))
            }
        }
        Statement::Merge(m) => dml(DmlKind::Merge).target(table_factor_name(&m.table)),
        Statement::LoadData { table_name, .. } => dml(DmlKind::LoadData).target(Some(table_name)),
        Statement::Copy { to: false, .. } => dml(DmlKind::Copy),
        Statement::Copy {
            target: CopyTarget::Stdout,
            ..
        } => Classified::new(C::Read),
        // COPY ... TO file/program writes outside the database.
        Statement::Copy { .. } => Classified::new(C::Unknown),
        Statement::Explain {
            analyze,
            options,
            statement,
            ..
        } if *analyze
            || options.iter().flatten().any(|o| {
                o.name.value.eq_ignore_ascii_case("analyze")
                    || o.name.value.eq_ignore_ascii_case("analyse")
            }) =>
        {
            Classified {
                cursorable: false,
                ..of_stmt(statement)
            }
        }
        Statement::Explain { .. }
        | Statement::ExplainTable { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCatalogs { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowProcessList { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowObjects(_)
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowCollation { .. } => Classified::new(C::Read),
        Statement::Set(_) | Statement::Use(_) => Classified::new(C::Session),
        Statement::StartTransaction { .. } => Classified::new(C::Tx(TxOp::Begin)),
        // The transaction stays open: ROLLBACK TO SAVEPOINT, AND CHAIN.
        Statement::Rollback {
            savepoint: Some(_), ..
        }
        | Statement::Commit { chain: true, .. }
        | Statement::Rollback { chain: true, .. } => Classified::new(C::Tx(TxOp::Savepoint)),
        Statement::Commit { .. } => Classified::new(C::Tx(TxOp::Commit)),
        Statement::Rollback { .. } => Classified::new(C::Tx(TxOp::Rollback)),
        Statement::Savepoint { .. } | Statement::ReleaseSavepoint { .. } => {
            Classified::new(C::Tx(TxOp::Savepoint))
        }
        Statement::Drop {
            object_type, names, ..
        } => destructive(if *object_type == ObjectType::Table {
            D::DropTable
        } else {
            D::DropOther
        })
        .target(names.first()),
        Statement::Truncate(t) => {
            destructive(D::Truncate).target(t.table_names.first().map(|t| &t.name))
        }
        Statement::AlterTable(a) => destructive(D::Alter).target(Some(&a.name)),
        Statement::RenameTable(r) => destructive(D::Rename).target(r.first().map(|r| &r.old_name)),
        Statement::Grant(_) => destructive(D::Grant),
        Statement::Revoke(_) => destructive(D::Revoke),
        Statement::CreateTable(t) if !t.or_replace => {
            Classified::new(C::CreateSafe).target(Some(&t.name))
        }
        // Remaining CREATE/ALTER/DROP variants are told apart by their canonical rendering.
        _ => {
            let text = stmt.to_string().to_ascii_uppercase();
            if text.starts_with("CREATE OR REPLACE") {
                destructive(D::CreateOrReplace)
            } else if text.starts_with("ALTER") {
                destructive(D::Alter)
            } else if text.starts_with("DROP") {
                destructive(D::DropOther)
            } else if text.starts_with("CREATE") {
                Classified::new(C::CreateSafe)
            } else {
                Classified::new(C::Unknown)
            }
        }
    }
}

#[cfg(test)]
mod tests;
