//! SQL autocomplete popup state and the per-workspace column cache (no GUI).
use std::collections::HashMap;
use std::ops::Range;

use crustdb_core::model::{DriverKind, ObjectKind, ObjectRef};
use crustdb_core::sql::{CompletionContext, Expect, TokenKind, highlight, keywords, rank};
use eframe::egui::Key;

use super::tree::{Load, ObjectTree};
use crate::runtime::RequestId;

pub const MAX_ITEMS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutocompleteAction {
    None,
    /// Replacement text and the byte range of the typed prefix it replaces.
    Accept(String, Range<usize>),
    Close,
}

#[derive(Debug, Default)]
pub struct Autocomplete {
    pub open: bool,
    pub items: Vec<String>,
    pub selected: usize,
    pub replace: Range<usize>,
}

/// Column names by table, filled through hidden `OpenTable` + `LoadStructure` round trips.
// ponytail: `ObjectRef` is not `Hash`; linear scans are fine for a handful of tables.
#[derive(Debug, Default)]
pub struct ColumnCache {
    cols: Vec<(ObjectRef, Vec<String>)>,
    asked: Vec<ObjectRef>,
    pending: HashMap<RequestId, ObjectRef>,
}

fn last_part(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn is_table(o: &ObjectRef) -> bool {
    matches!(
        o.kind,
        ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView
    )
}

fn tables(tree: &ObjectTree) -> impl Iterator<Item = &ObjectRef> {
    tree.entries
        .values()
        .filter_map(|e| match &e.load {
            Load::Done(v) => Some(v),
            _ => None,
        })
        .flatten()
        .filter(|o| is_table(o))
}

impl ColumnCache {
    pub fn insert(&mut self, object: ObjectRef, columns: Vec<String>) {
        self.cols.retain(|(o, _)| *o != object);
        self.cols.push((object, columns));
    }

    /// Columns of the cached table called `name` (`schema.table` matches on the last part).
    pub fn columns_of(&self, name: &str) -> Vec<&str> {
        let name = last_part(name);
        self.cols
            .iter()
            .filter(|(o, _)| o.name == name)
            .flat_map(|(_, c)| c.iter().map(String::as_str))
            .collect()
    }

    /// Tables the context refers to that the tree knows but nothing has asked for yet.
    pub fn wanted(&self, ctx: &CompletionContext, tree: &ObjectTree) -> Vec<ObjectRef> {
        let mut out: Vec<ObjectRef> = vec![];
        for t in &ctx.referenced_tables {
            if let Some(o) = tables(tree).find(|o| o.name == last_part(&t.name))
                && !self.asked.contains(o)
                && !self.cols.iter().any(|(c, _)| c == o)
                && !out.contains(o)
            {
                out.push(o.clone());
            }
        }
        out
    }

    pub fn asked(&self, object: &ObjectRef) -> bool {
        self.asked.contains(object)
    }

    pub fn begin(&mut self, object: ObjectRef, req: RequestId) {
        self.asked.push(object.clone());
        self.pending.insert(req, object);
    }

    /// Stores the reply of a `begin`; a failed lookup (`None`) is forgotten so it can be asked again.
    pub fn apply(&mut self, req: RequestId, columns: Option<Vec<String>>) {
        let Some(object) = self.pending.remove(&req) else {
            return;
        };
        match columns {
            Some(c) => self.insert(object, c),
            None => self.asked.retain(|o| *o != object),
        }
    }
}

/// True when `byte` sits inside a string literal or a comment (no popup there).
pub fn in_literal_or_comment(text: &str, byte: usize, kind: DriverKind) -> bool {
    highlight(text, kind).iter().any(|(r, k)| {
        let done = match k {
            TokenKind::String => {
                text[r.clone()].len() > 1 && text[r.clone()].ends_with(['\'', '"'])
            }
            TokenKind::Comment => {
                let t = &text[r.clone()];
                t.ends_with("*/") || t.ends_with('\n')
            }
            _ => return false,
        };
        r.start < byte && (byte < r.end || (byte == r.end && !done))
    })
}

impl Autocomplete {
    pub fn refresh(&mut self, ctx: &CompletionContext, tree: &ObjectTree, columns: &ColumnCache) {
        let dotted = matches!(ctx.expect, Expect::Column { qualifier: Some(_) });
        if ctx.prefix.is_empty() && !dotted {
            return self.close();
        }
        let driver = tree.driver;
        let mut pool: Vec<String> = vec![];
        let kws = || keywords(driver).iter().map(|k| (*k).to_owned());
        let table_names = || {
            let mut v: Vec<String> = vec![];
            for o in tables(tree) {
                if !v.contains(&o.name) {
                    v.push(o.name.clone());
                }
            }
            v
        };
        let col_names = |qualifier: Option<&str>| {
            let mut v: Vec<String> = vec![];
            for t in &ctx.referenced_tables {
                let hit = qualifier.is_none_or(|q| {
                    t.alias.as_deref() == Some(q) || t.name == q || last_part(&t.name) == q
                });
                if hit {
                    v.extend(columns.columns_of(&t.name).into_iter().map(str::to_owned));
                }
            }
            v
        };
        match &ctx.expect {
            Expect::Keyword => pool.extend(kws()),
            Expect::Table => pool.extend(table_names()),
            Expect::Column { qualifier } => {
                pool.extend(col_names(qualifier.as_deref()));
                if qualifier.is_none() {
                    pool.extend(kws());
                }
            }
            Expect::Any => {
                pool.extend(table_names());
                pool.extend(kws());
            }
        }
        pool.dedup();
        let refs: Vec<&str> = pool.iter().map(String::as_str).collect();
        self.items = rank(&refs, &ctx.prefix, MAX_ITEMS)
            .into_iter()
            .map(|i| pool[i].clone())
            .collect();
        self.open = !self.items.is_empty();
        self.selected = 0;
        self.replace = ctx.prefix_range.clone();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
    }

    pub fn on_key(&mut self, key: Key) -> AutocompleteAction {
        if !self.open {
            return AutocompleteAction::None;
        }
        match key {
            Key::ArrowUp => self.selected = self.selected.saturating_sub(1),
            Key::ArrowDown => self.selected = (self.selected + 1).min(self.items.len() - 1),
            Key::Enter | Key::Tab => {
                let item = self.items[self.selected].clone();
                let range = self.replace.clone();
                self.close();
                return AutocompleteAction::Accept(item, range);
            }
            Key::Escape => {
                self.close();
                return AutocompleteAction::Close;
            }
            _ => {}
        }
        AutocompleteAction::None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use crustdb_core::model::DriverKind;
    use crustdb_core::sql::completion_context;

    use super::*;
    use crate::runtime::{Command, WorkspaceId};

    const MY: DriverKind = DriverKind::Mysql;

    fn obj(name: &str, kind: ObjectKind) -> ObjectRef {
        ObjectRef {
            database: "app".into(),
            schema: None,
            name: name.into(),
            kind,
        }
    }

    /// MySQL tree with the given tables loaded under `app`.
    fn tree(tables: &[&str]) -> ObjectTree {
        let mut t = ObjectTree::new(WorkspaceId(1), MY);
        let req = |c: Option<Command>| match c {
            Some(Command::LoadChildren { req, .. }) => req,
            other => panic!("{other:?}"),
        };
        let r = req(t.load_roots());
        t.apply_children(r, vec![obj("app", ObjectKind::Database)]);
        let folder = crate::state::tree::NodePath::Folder {
            database: "app".into(),
            schema: None,
            kind: ObjectKind::Table,
        };
        let r = req(t.expand(folder));
        t.apply_children(
            r,
            tables.iter().map(|n| obj(n, ObjectKind::Table)).collect(),
        );
        t
    }

    fn refreshed(text: &str, tree: &ObjectTree, cols: &ColumnCache) -> Autocomplete {
        let off = text.find('|').unwrap();
        let ctx = completion_context(&text.replace('|', ""), off, MY);
        let mut ac = Autocomplete::default();
        ac.refresh(&ctx, tree, cols);
        ac
    }

    #[test]
    fn popup_opens_after_one_identifier_char() {
        // Arrange
        let t = tree(&[]);
        let c = ColumnCache::default();

        // Act & Assert
        assert!(!refreshed("|", &t, &c).open);
        let ac = refreshed("SEL|", &t, &c);
        assert!(ac.open);
        assert_eq!(ac.items[0], "SELECT");
        assert_eq!(ac.replace, 0..3);
    }

    #[test]
    fn arrow_enter_tab_esc_behaviour() {
        // Arrange
        let t = tree(&[]);

        // Act & Assert
        let mut ac = refreshed("S|", &t, &ColumnCache::default());
        assert!(ac.items.len() > 2);
        assert_eq!(ac.on_key(Key::ArrowUp), AutocompleteAction::None);
        assert_eq!(ac.selected, 0, "clamped at the top");
        ac.on_key(Key::ArrowDown);
        assert_eq!(ac.selected, 1);
        let want = ac.items[1].clone();
        assert_eq!(
            ac.on_key(Key::Enter),
            AutocompleteAction::Accept(want, 0..1)
        );
        assert!(!ac.open);
        let mut ac = refreshed("S|", &t, &ColumnCache::default());
        assert!(matches!(
            ac.on_key(Key::Tab),
            AutocompleteAction::Accept(..)
        ));
        let mut ac = refreshed("S|", &t, &ColumnCache::default());
        assert_eq!(ac.on_key(Key::Escape), AutocompleteAction::Close);
        assert!(!ac.open);
        assert_eq!(
            ac.on_key(Key::Enter),
            AutocompleteAction::None,
            "closed popup ignores keys"
        );
    }

    #[test]
    fn table_suggestions_after_from() {
        // Arrange
        let t = tree(&["users", "orders", "audit_log"]);

        // Act & Assert
        let ac = refreshed("SELECT * FROM us|", &t, &ColumnCache::default());
        assert_eq!(ac.items[0], "users");
        let ac = refreshed("SELECT * FROM |x", &t, &ColumnCache::default());
        assert!(!ac.open, "no prefix, no popup");
    }

    #[test]
    fn column_suggestions_for_alias() {
        // Arrange
        let t = tree(&["users"]);
        let mut cols = ColumnCache::default();
        cols.insert(
            obj("users", ObjectKind::Table),
            vec!["id".into(), "name".into()],
        );

        // Act & Assert
        let ac = refreshed("SELECT u.| FROM users u", &t, &cols);
        assert!(ac.open, "a dot opens it without a prefix");
        assert_eq!(ac.items, ["id", "name"]);
        let ac = refreshed("SELECT na| FROM users u", &t, &cols);
        assert_eq!(ac.items[0], "name");
        let ctx = completion_context("SELECT u.id FROM users u", 8, MY);
        assert_eq!(cols.wanted(&ctx, &tree(&["users"])), []);
        let ctx = completion_context("SELECT id FROM users", 8, MY);
        assert_eq!(
            ColumnCache::default().wanted(&ctx, &tree(&["users"])),
            [obj("users", ObjectKind::Table)]
        );
    }

    #[test]
    fn max_50_items() {
        // Arrange
        let names: Vec<String> = (0..80).map(|i| format!("t{i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();

        // Act
        let ac = refreshed("SELECT * FROM t|", &tree(&refs), &ColumnCache::default());

        // Assert
        assert_eq!(ac.items.len(), MAX_ITEMS);
    }

    #[test]
    fn no_popup_inside_string_or_comment() {
        // Arrange
        let off = |t: &str| t.find('|').unwrap();

        // Act & Assert
        for text in [
            "SELECT 'ab|c'",
            "SELECT 1 -- fo|",
            "SELECT /* fo| */ 1",
            "SELECT 'ab|",
        ] {
            let t = text.replace('|', "");
            assert!(in_literal_or_comment(&t, off(text), MY), "{text}");
        }
        for text in [
            "SELECT 'abc'|",
            "SELECT 1 -- c\nSE|",
            "SELECT /* c */|",
            "SEL|",
        ] {
            let t = text.replace('|', "");
            assert!(!in_literal_or_comment(&t, off(text), MY), "{text}");
        }
    }

    #[test]
    fn failed_column_lookup_can_be_asked_again() {
        // Arrange
        let t = tree(&["users"]);
        let ctx = completion_context("SELECT id FROM users", 8, MY);
        let mut cols = ColumnCache::default();
        let users = obj("users", ObjectKind::Table);

        // Act & Assert
        cols.begin(users.clone(), RequestId(1));
        assert!(cols.wanted(&ctx, &t).is_empty(), "in flight");
        cols.apply(RequestId(1), None);
        assert_eq!(cols.wanted(&ctx, &t), std::slice::from_ref(&users));
        cols.begin(users.clone(), RequestId(2));
        cols.apply(RequestId(2), Some(vec!["id".into()]));
        assert_eq!(cols.columns_of("users"), ["id"]);
        assert!(cols.wanted(&ctx, &t).is_empty());
    }
}
