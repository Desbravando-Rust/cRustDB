//! Lazy object tree of one workspace (spec §4.2): children load on first expand and are cached
//! per node until refreshed.
use std::collections::HashMap;

use crustdb_core::model::{DriverKind, ObjectKind, ObjectRef};

use crate::runtime::{Command, RequestId, WorkspaceId};

/// Request ids minted here live above the runtime's counter and are seeded per workspace, since
/// `Event::Children` carries no workspace and is offered to every tree.
const REQ_BASE: u64 = 1 << 63;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodePath {
    Database(String),
    Schema(String, String),
    Folder {
        database: String,
        schema: Option<String>,
        kind: ObjectKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibleNode {
    pub depth: usize,
    pub label: String,
    /// `None` for leaves.
    pub path: Option<NodePath>,
    /// Set for leaves.
    pub object: Option<ObjectRef>,
    pub expanded: bool,
    pub loading: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAction {
    Data,
    Ddl,
}

/// Double click: tables and views open data; everything else opens the DDL.
pub fn open_action(kind: ObjectKind) -> OpenAction {
    match kind {
        ObjectKind::Table | ObjectKind::View | ObjectKind::MaterializedView => OpenAction::Data,
        _ => OpenAction::Ddl,
    }
}

pub fn folder_kinds(driver: DriverKind) -> &'static [ObjectKind] {
    use ObjectKind::*;
    match driver {
        DriverKind::Postgres => &[
            Table,
            View,
            MaterializedView,
            Function,
            Procedure,
            Trigger,
            Sequence,
            Type,
        ],
        DriverKind::Mysql => &[Table, View, Function, Procedure, Trigger],
        DriverKind::Sqlite => &[Table, View, Trigger],
    }
}

fn folder_label(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Table => "Tables",
        ObjectKind::View => "Views",
        ObjectKind::MaterializedView => "Materialized Views",
        ObjectKind::Function => "Functions",
        ObjectKind::Procedure => "Procedures",
        ObjectKind::Trigger => "Triggers",
        ObjectKind::Sequence => "Sequences",
        ObjectKind::Type => "Types",
        ObjectKind::Database => "Databases",
        ObjectKind::Schema => "Schemas",
    }
}

#[derive(Debug, Default, Clone)]
pub(super) enum Load {
    #[default]
    Idle,
    Pending(RequestId),
    Done(Vec<ObjectRef>),
}

#[derive(Debug, Default)]
pub(super) struct Entry {
    expanded: bool,
    pub(super) load: Load,
}

#[derive(Debug)]
pub struct ObjectTree {
    ws: WorkspaceId,
    pub(super) driver: DriverKind,
    root: Load,
    pub(super) entries: HashMap<NodePath, Entry>,
    filter: String,
    pub selected: Option<NodePath>,
    next_req: u64,
}

impl ObjectTree {
    pub fn new(ws: WorkspaceId, driver: DriverKind) -> Self {
        Self {
            ws,
            driver,
            root: Load::Idle,
            entries: HashMap::new(),
            filter: String::new(),
            selected: None,
            // ponytail: 32 bits of counter per workspace and ws ids below 2^31; widen if ever hit.
            next_req: REQ_BASE | (ws.0 << 32),
        }
    }

    pub fn filter_text(&self) -> &str {
        &self.filter
    }

    pub fn filter(&mut self, q: &str) {
        self.filter = q.to_owned();
    }

    fn command(
        &mut self,
        database: String,
        schema: Option<String>,
        kind: ObjectKind,
    ) -> (RequestId, Command) {
        let req = RequestId(self.next_req);
        self.next_req += 1;
        let cmd = Command::LoadChildren {
            ws: self.ws,
            req,
            database,
            schema,
            kind: Some(kind),
        };
        (req, cmd)
    }

    /// Drops every cached node; request ids keep counting so stale replies stay unmatched.
    pub fn reset(&mut self) {
        self.root = Load::Idle;
        self.entries.clear();
    }

    pub fn refresh_root(&mut self) -> Option<Command> {
        self.reset();
        self.load_roots()
    }

    /// Requests the database list once.
    pub fn load_roots(&mut self) -> Option<Command> {
        if !matches!(self.root, Load::Idle) {
            return None;
        }
        let (req, cmd) = self.command(String::new(), None, ObjectKind::Database);
        self.root = Load::Pending(req);
        Some(cmd)
    }

    /// Marks the node loading and builds its command; `None` for nodes that need no load
    /// (PG schemas and MySQL databases only list the static folders).
    fn request(&mut self, node: &NodePath) -> Option<Command> {
        let (cmd, req) = match node {
            NodePath::Database(d) if self.driver == DriverKind::Postgres => {
                let (r, c) = self.command(d.clone(), None, ObjectKind::Schema);
                (c, r)
            }
            NodePath::Folder {
                database,
                schema,
                kind,
            } => {
                let (r, c) = self.command(database.clone(), schema.clone(), *kind);
                (c, r)
            }
            _ => return None,
        };
        self.entries.entry(node.clone()).or_default().load = Load::Pending(req);
        Some(cmd)
    }

    pub fn expand(&mut self, node: NodePath) -> Option<Command> {
        let e = self.entries.entry(node.clone()).or_default();
        e.expanded = true;
        if matches!(e.load, Load::Idle) {
            self.request(&node)
        } else {
            None
        }
    }

    pub fn toggle(&mut self, node: NodePath) -> Option<Command> {
        if self.entries.get(&node).is_some_and(|e| e.expanded) {
            self.entries.entry(node).or_default().expanded = false;
            None
        } else {
            self.expand(node)
        }
    }

    /// Drops the cache below `node` and reloads the node itself, keeping it expanded.
    pub fn refresh(&mut self, node: NodePath) -> Option<Command> {
        self.entries
            .retain(|k, _| k == &node || !is_below(&node, k));
        if let Some(e) = self.entries.get_mut(&node) {
            e.load = Load::Idle;
        }
        self.request(&node)
    }

    pub fn apply_children(&mut self, req: RequestId, items: Vec<ObjectRef>) {
        if matches!(self.root, Load::Pending(r) if r == req) {
            self.root = Load::Done(items);
        } else if let Some(e) = self
            .entries
            .values_mut()
            .find(|e| matches!(e.load, Load::Pending(r) if r == req))
        {
            e.load = Load::Done(items);
        }
    }

    fn items(&self, node: &NodePath) -> &[ObjectRef] {
        match self.entries.get(node) {
            Some(Entry {
                load: Load::Done(v),
                ..
            }) => v,
            _ => &[],
        }
    }

    fn matches(&self, label: &str) -> bool {
        label.to_lowercase().contains(&self.filter.to_lowercase())
    }

    /// A container row followed by its children; with a filter active, only branches that hold a
    /// loaded match (or match themselves) survive and they show expanded.
    fn rows(&self, path: NodePath, label: &str, depth: usize) -> Vec<VisibleNode> {
        let entry = self.entries.get(&path);
        let expanded = entry.is_some_and(|e| e.expanded);
        let filtering = !self.filter.is_empty();
        let mut children = vec![];
        if expanded || filtering {
            match &path {
                NodePath::Database(d) if self.driver == DriverKind::Postgres => {
                    for s in self.items(&path) {
                        children.extend(self.rows(
                            NodePath::Schema(d.clone(), s.name.clone()),
                            &s.name,
                            depth + 1,
                        ));
                    }
                }
                NodePath::Database(d) => children.extend(self.folders(d, None, depth + 1)),
                NodePath::Schema(d, s) => children.extend(self.folders(d, Some(s), depth + 1)),
                NodePath::Folder { .. } => {
                    for o in self.items(&path) {
                        if !filtering || self.matches(&o.name) {
                            children.push(VisibleNode {
                                depth: depth + 1,
                                label: o.name.clone(),
                                path: None,
                                object: Some(o.clone()),
                                expanded: false,
                                loading: false,
                            });
                        }
                    }
                }
            }
        }
        if filtering && children.is_empty() && !self.matches(label) {
            return vec![];
        }
        let mut out = vec![VisibleNode {
            depth,
            label: label.to_owned(),
            path: Some(path),
            object: None,
            expanded,
            loading: matches!(
                entry,
                Some(Entry {
                    load: Load::Pending(_),
                    ..
                })
            ),
        }];
        out.extend(children);
        out
    }

    fn folders(&self, database: &str, schema: Option<&String>, depth: usize) -> Vec<VisibleNode> {
        folder_kinds(self.driver)
            .iter()
            .flat_map(|&kind| {
                let path = NodePath::Folder {
                    database: database.to_owned(),
                    schema: schema.cloned(),
                    kind,
                };
                self.rows(path, folder_label(kind), depth)
            })
            .collect()
    }

    pub fn visible(&self) -> impl Iterator<Item = VisibleNode> {
        let dbs = match &self.root {
            Load::Done(v) => v.as_slice(),
            _ => &[],
        };
        dbs.iter()
            .flat_map(|d| self.rows(NodePath::Database(d.name.clone()), &d.name, 0))
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// Whether `key` is a descendant of `node` in the cache.
fn is_below(node: &NodePath, key: &NodePath) -> bool {
    match (node, key) {
        (NodePath::Database(d), NodePath::Schema(kd, _)) => d == kd,
        (NodePath::Database(d), NodePath::Folder { database, .. }) => d == database,
        (
            NodePath::Schema(d, s),
            NodePath::Folder {
                database, schema, ..
            },
        ) => d == database && schema.as_ref() == Some(s),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn obj(db: &str, schema: Option<&str>, name: &str, kind: ObjectKind) -> ObjectRef {
        ObjectRef {
            database: db.into(),
            schema: schema.map(Into::into),
            name: name.into(),
            kind,
        }
    }

    fn req_of(cmd: Option<Command>) -> RequestId {
        match cmd {
            Some(Command::LoadChildren { req, .. }) => req,
            other => panic!("expected LoadChildren, got {other:?}"),
        }
    }

    fn tree(driver: DriverKind) -> ObjectTree {
        let mut t = ObjectTree::new(WorkspaceId(1), driver);
        let r = req_of(t.load_roots());
        t.apply_children(r, vec![obj("app", None, "app", ObjectKind::Database)]);
        t
    }

    fn folder(schema: Option<&str>, kind: ObjectKind) -> NodePath {
        NodePath::Folder {
            database: "app".into(),
            schema: schema.map(Into::into),
            kind,
        }
    }

    fn labels(t: &ObjectTree) -> Vec<(usize, String)> {
        t.visible().map(|n| (n.depth, n.label)).collect()
    }

    #[test]
    fn pg_tree_has_schema_level_mysql_does_not() {
        // Arrange
        let mut pg = tree(DriverKind::Postgres);

        // Act & Assert
        let r = req_of(pg.expand(NodePath::Database("app".into())));
        pg.apply_children(r, vec![obj("app", None, "public", ObjectKind::Schema)]);
        assert_eq!(labels(&pg), [(0, "app".into()), (1, "public".into())]);
        assert!(
            pg.expand(NodePath::Schema("app".into(), "public".into()))
                .is_none()
        );
        let v = labels(&pg);
        assert_eq!(v[2], (2, "Tables".into()));

        let mut my = tree(DriverKind::Mysql);
        assert!(my.expand(NodePath::Database("app".into())).is_none());
        assert_eq!(labels(&my)[1], (1, "Tables".into()));
    }

    #[test]
    fn two_trees_never_share_request_ids() {
        // Arrange
        let mut a = ObjectTree::new(WorkspaceId(1), DriverKind::Mysql);
        let mut b = ObjectTree::new(WorkspaceId(2), DriverKind::Mysql);

        // Act & Assert
        let ra = req_of(a.load_roots());
        let rb = req_of(b.load_roots());
        assert_ne!(ra, rb);
        b.apply_children(ra, vec![obj("x", None, "x", ObjectKind::Database)]);
        assert!(b.visible().next().is_none(), "foreign reply ignored");
    }

    #[test]
    fn empty_children_resolves_pending_and_root_refreshes() {
        // Arrange
        let mut t = ObjectTree::new(WorkspaceId(1), DriverKind::Mysql);

        // Act & Assert
        let r = req_of(t.load_roots());
        t.apply_children(r, vec![]);
        assert!(t.load_roots().is_none(), "empty reply resolved the root");
        let r2 = req_of(t.refresh_root());
        assert_ne!(r, r2);
        t.apply_children(r2, vec![obj("app", None, "app", ObjectKind::Database)]);
        assert_eq!(t.visible().count(), 1);
        t.reset();
        assert!(t.load_roots().is_some(), "reconnect starts over");
    }

    #[test]
    fn filter_keeps_original_text() {
        // Arrange
        let mut t = ObjectTree::new(WorkspaceId(1), DriverKind::Mysql);

        // Act
        t.filter("ORD");

        // Assert
        assert_eq!(t.filter_text(), "ORD");
    }

    #[test]
    fn folders_match_spec_per_driver() {
        // Arrange
        use ObjectKind::*;

        // Act & Assert
        assert_eq!(
            folder_kinds(DriverKind::Postgres),
            [
                Table,
                View,
                MaterializedView,
                Function,
                Procedure,
                Trigger,
                Sequence,
                Type
            ]
        );
        assert_eq!(
            folder_kinds(DriverKind::Mysql),
            [Table, View, Function, Procedure, Trigger]
        );
    }

    #[test]
    fn expand_loads_once_and_caches() {
        // Arrange
        let mut t = tree(DriverKind::Mysql);
        t.expand(NodePath::Database("app".into()));
        let f = folder(None, ObjectKind::Table);

        // Act & Assert
        let r = req_of(t.expand(f.clone()));
        assert!(t.expand(f.clone()).is_none(), "pending");
        t.apply_children(r, vec![obj("app", None, "users", ObjectKind::Table)]);
        assert!(t.toggle(f.clone()).is_none(), "collapse");
        assert!(t.toggle(f.clone()).is_none(), "re-expand uses the cache");
        assert!(t.visible().any(|n| n.label == "users"));
        let r2 = req_of(t.refresh(f));
        assert_ne!(r, r2);
    }

    #[test]
    fn filter_matches_loaded_nodes_case_insensitive() {
        // Arrange
        let mut t = tree(DriverKind::Mysql);
        t.expand(NodePath::Database("app".into()));
        let r = req_of(t.expand(folder(None, ObjectKind::Table)));
        t.apply_children(
            r,
            vec![
                obj("app", None, "users", ObjectKind::Table),
                obj("app", None, "Orders", ObjectKind::Table),
            ],
        );

        // Act & Assert
        t.filter("ORD");
        let names: Vec<_> = t.visible().map(|n| n.label).collect();
        assert_eq!(names, ["app", "Tables", "Orders"]);
        t.filter("");
        assert_eq!(t.visible().filter(|n| n.object.is_some()).count(), 2);
    }

    #[test]
    fn double_click_table_opens_data_tab_function_opens_ddl() {
        // Arrange
        use ObjectKind::*;

        // Act & Assert
        for k in [Table, View, MaterializedView] {
            assert_eq!(open_action(k), OpenAction::Data);
        }
        for k in [Function, Procedure, Trigger, Sequence, Type] {
            assert_eq!(open_action(k), OpenAction::Ddl);
        }
    }
}
