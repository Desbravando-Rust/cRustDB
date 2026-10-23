mod beautify;
mod classify;
mod complete;
mod highlight;
mod keywords;
mod split;

pub use beautify::beautify;
pub use classify::{
    Classified, DestructiveKind, DmlKind, StatementClass, TargetName, TxOp, classify,
};

pub use complete::{CompletionContext, Expect, TableAlias, completion_context, rank};
pub use highlight::{TokenKind, highlight};
pub use keywords::keywords;
pub use split::{StatementSpan, split, statement_at, statements_in};

use std::cell::Cell;

use sqlparser::tokenizer::Location;

/// Maps sqlparser `Location`s (1-based line, 1-based column counted in chars) to byte offsets.
pub(crate) struct LineIndex<'a> {
    text: &'a str,
    starts: Vec<usize>,
    /// Last resolved (line index, column, byte offset); queries are mostly increasing.
    last: Cell<(usize, usize, usize)>,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self {
            text,
            starts,
            last: Cell::new((0, 0, 0)),
        }
    }

    pub(crate) fn offset(&self, loc: Location) -> usize {
        let line = (loc.line as usize)
            .saturating_sub(1)
            .min(self.starts.len() - 1);
        let col = (loc.column as usize).saturating_sub(1);
        let (ll, lc, lb) = self.last.get();
        let (mut c, mut b) = if ll == line && lc <= col {
            (lc, lb)
        } else {
            (0, self.starts[line])
        };
        let mut chars = self.text[b..].chars();
        while c < col {
            match chars.next() {
                Some('\n') | None => break,
                Some(ch) => {
                    b += ch.len_utf8();
                    c += 1;
                }
            }
        }
        self.last.set((line, c, b));
        b
    }
}

pub(crate) fn dialect(kind: crate::model::DriverKind) -> &'static dyn sqlparser::dialect::Dialect {
    match kind {
        crate::model::DriverKind::Postgres => &sqlparser::dialect::PostgreSqlDialect {},
        crate::model::DriverKind::Mysql => &sqlparser::dialect::MySqlDialect {},
        crate::model::DriverKind::Sqlite => &sqlparser::dialect::SQLiteDialect {},
    }
}
