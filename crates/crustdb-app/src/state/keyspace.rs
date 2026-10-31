//! Keyspace of a key-value workspace: lazy scan batches, a key tree and the open entry.
use std::collections::HashSet;
use std::time::Duration;

use crustdb_core::model::{ColumnMeta, Row};
use crustdb_core::store::keyspace::{EntryKey, KeyNode, group};
use crustdb_core::store::{EntryView, ValueKind};

use super::key_scan::KeyScan;
use super::{AppState, Status};
use crate::runtime::{Command, Event, RequestId, WorkspaceId};

const SEP: &str = ":";

/// One visible row of the key tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatNode {
    pub depth: usize,
    pub label: String,
    /// Prefix bytes up to this node; identifies it for `toggle`.
    pub path: Vec<u8>,
    pub key: Option<EntryKey>,
    pub branch: bool,
    pub expanded: bool,
}

/// The value of the opened key, shown read-only in the grid.
#[derive(Debug)]
pub struct EntryPane {
    pub key: EntryKey,
    pub kind: Option<ValueKind>,
    pub ttl: Option<Duration>,
    pub size_bytes: Option<u64>,
    pub truncated: bool,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    pub anchor: Option<usize>,
}

#[derive(Debug)]
pub struct Keyspace {
    ws: WorkspaceId,
    pub namespaces: Vec<String>,
    ns_req: Option<RequestId>,
    pub scan: KeyScan,
    tree: Vec<KeyNode>,
    expanded: HashSet<Vec<u8>>,
    flat: Vec<FlatNode>,
    pub error: Option<String>,
    entry_req: Option<RequestId>,
    pub entry: Option<EntryPane>,
}

impl Keyspace {
    pub fn new(ws: WorkspaceId) -> Self {
        Self {
            ws,
            namespaces: vec![],
            ns_req: None,
            scan: KeyScan::new(None),
            tree: vec![],
            expanded: HashSet::new(),
            flat: vec![],
            error: None,
            entry_req: None,
            entry: None,
        }
    }

    /// Visible rows of the tree, in display order.
    pub fn nodes(&self) -> &[FlatNode] {
        &self.flat
    }

    /// Namespaces plus the first batch of keys.
    pub fn start(&mut self, ns_req: RequestId, scan_req: RequestId) -> Vec<Command> {
        self.ns_req = Some(ns_req);
        let list = Command::ListNamespaces {
            ws: self.ws,
            req: ns_req,
        };
        std::iter::once(list)
            .chain(self.scan.next(self.ws, scan_req))
            .collect()
    }

    /// Next batch, only when asked; asking again after a failure reuses the same cursor.
    pub fn load_more(&mut self, req: RequestId) -> Option<Command> {
        let cmd = self.scan.next(self.ws, req);
        if cmd.is_some() {
            self.error = None;
        }
        cmd
    }

    /// Restarts the scan with a glob pattern (empty clears it); older answers are dropped.
    pub fn set_filter(&mut self, text: String, req: RequestId) -> Option<Command> {
        let pattern = (!text.trim().is_empty()).then_some(text);
        self.scan = KeyScan::new(pattern);
        self.error = None;
        self.rebuild();
        self.scan.next(self.ws, req)
    }

    pub fn open(&mut self, key: EntryKey, req: RequestId) -> Command {
        self.entry_req = Some(req);
        self.error = None;
        let kind = self
            .scan
            .entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| e.kind.clone());
        self.entry = Some(EntryPane {
            key: key.clone(),
            kind,
            ttl: None,
            size_bytes: None,
            truncated: false,
            columns: vec![],
            rows: vec![],
            selected: None,
            anchor: None,
        });
        Command::ReadEntry {
            ws: self.ws,
            req,
            key,
        }
    }

    /// Expands or collapses the node at `path`.
    pub fn toggle(&mut self, path: &[u8]) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_vec());
        }
        self.reflatten();
    }

    pub fn apply(&mut self, e: &Event) {
        match e {
            Event::Namespaces { req, result } if self.ns_req == Some(*req) => {
                self.ns_req = None;
                match result {
                    Ok(n) => self.namespaces.clone_from(n),
                    Err(m) => self.error = Some(m.clone()),
                }
            }
            Event::KeysScanned { req, result } if self.scan.is_pending(*req) => match result {
                Ok(page) => {
                    self.error = None;
                    self.scan.apply(*req, page.clone());
                    self.rebuild();
                }
                Err(m) => {
                    self.scan.fail(*req);
                    self.error = Some(m.clone());
                }
            },
            Event::EntryRead { req, result } if self.entry_req == Some(*req) => {
                self.entry_req = None;
                match result {
                    Ok(view) => self.fill_entry(view),
                    Err(m) => {
                        self.entry = None;
                        self.error = Some(m.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn fill_entry(&mut self, view: &EntryView) {
        let Some(p) = &mut self.entry else { return };
        let (columns, rows) = view.body.to_rows();
        p.ttl = view.ttl;
        p.size_bytes = view.size_bytes;
        p.truncated = view.truncated;
        p.columns = columns;
        p.rows = rows;
    }

    /// Regroups every key; runs once per arriving batch, never per frame.
    // ponytail: regroups all loaded keys per batch (O(n) per batch); make it incremental if slow.
    fn rebuild(&mut self) {
        let keys: Vec<EntryKey> = self.scan.entries.iter().map(|e| e.key.clone()).collect();
        self.tree = group(&keys, SEP);
        self.reflatten();
    }

    fn reflatten(&mut self) {
        let mut out = vec![];
        flatten(&self.tree, 0, &[], &self.expanded, &mut out);
        self.flat = out;
    }
}

fn flatten(
    nodes: &[KeyNode],
    depth: usize,
    prefix: &[u8],
    expanded: &HashSet<Vec<u8>>,
    out: &mut Vec<FlatNode>,
) {
    for n in nodes {
        let mut path = prefix.to_vec();
        if depth > 0 {
            path.extend_from_slice(SEP.as_bytes());
        }
        path.extend_from_slice(&n.segment);
        let open = expanded.contains(&path);
        out.push(FlatNode {
            depth,
            label: n.label().into_owned(),
            path: path.clone(),
            key: n.key.clone(),
            branch: !n.children.is_empty(),
            expanded: open,
        });
        if open {
            flatten(&n.children, depth + 1, &path, expanded, out);
        }
    }
}

impl AppState {
    /// Request id for commands issued from `apply`, which has no runtime to ask.
    fn internal_req(&mut self) -> RequestId {
        self.next_exec += 1;
        RequestId(self.next_exec)
    }

    /// Starts the keyspace of a freshly connected key-value workspace.
    pub(super) fn start_keyspace(&mut self, ws: WorkspaceId) {
        let (ns, scan) = (self.internal_req(), self.internal_req());
        let Some(w) = self.workspace_mut(ws) else {
            return;
        };
        if let Some(ks) = w.keyspace.as_mut() {
            *ks = Keyspace::new(ws);
            let cmds = ks.start(ns, scan);
            self.outbox.extend(cmds);
        }
    }

    pub(super) fn apply_keyspace(&mut self, e: &Event) {
        for w in &mut self.workspaces {
            if matches!(w.status, Status::Online(_))
                && let Some(ks) = w.keyspace.as_mut()
            {
                ks.apply(e);
            }
        }
    }
}

#[cfg(test)]
#[path = "keyspace_tests.rs"]
mod tests;
