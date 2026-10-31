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
    /// Why the last load of this node failed.
    pub error: Option<String>,
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
        DriverKind::Redis => &[],
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
    Failed(String),
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
    pub(super) root: Load,
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
    pub(super) fn request(&mut self, node: &NodePath) -> Option<Command> {
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
        if matches!(e.load, Load::Idle | Load::Failed(_)) {
            self.request(&node)
        } else {
            None
        }
    }

    pub fn toggle(&mut self, node: NodePath) -> Option<Command> {
        let e = self.entries.get(&node);
        // A failed node stays expanded; clicking it retries instead of collapsing.
        if e.is_some_and(|e| e.expanded && !matches!(e.load, Load::Failed(_))) {
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
        self.resolve(req, Load::Done(items));
    }

    pub fn fail_children(&mut self, req: RequestId, msg: String) {
        self.resolve(req, Load::Failed(msg));
    }

    fn resolve(&mut self, req: RequestId, to: Load) {
        if matches!(self.root, Load::Pending(r) if r == req) {
            self.root = to;
        } else if let Some(e) = self
            .entries
            .values_mut()
            .find(|e| matches!(e.load, Load::Pending(r) if r == req))
        {
            e.load = to;
        }
    }

    pub fn root_error(&self) -> Option<&str> {
        match &self.root {
            Load::Failed(m) => Some(m),
            _ => None,
        }
    }

    pub(super) fn items(&self, node: &NodePath) -> &[ObjectRef] {
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
                                error: None,
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
            error: match entry {
                Some(Entry {
                    load: Load::Failed(m),
                    ..
                }) => Some(m.clone()),
                _ => None,
            },
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
#[path = "tree_tests.rs"]
mod tests;
