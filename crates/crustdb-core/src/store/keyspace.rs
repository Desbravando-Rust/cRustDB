use std::borrow::Cow;
use std::collections::BTreeMap;

/// Raw key bytes; stores are not required to hold UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntryKey(pub Vec<u8>);

impl EntryKey {
    pub fn display(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
}

/// A node can be a key and a prefix at once (`a` and `a:b`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyNode {
    pub segment: Vec<u8>,
    pub key: Option<EntryKey>,
    pub children: Vec<KeyNode>,
}

impl KeyNode {
    pub fn label(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.segment)
    }
}

#[derive(Default)]
struct Builder {
    key: Option<EntryKey>,
    children: BTreeMap<Vec<u8>, Builder>,
}

impl Builder {
    fn into_nodes(children: BTreeMap<Vec<u8>, Builder>) -> Vec<KeyNode> {
        children
            .into_iter()
            .map(|(segment, b)| {
                let Builder { key, children } = b;
                KeyNode {
                    segment,
                    key,
                    children: Builder::into_nodes(children),
                }
            })
            .collect()
    }
}

/// Tree depth limit; the server controls key shape, so recursion must stay bounded.
const MAX_DEPTH: usize = 64;

/// Splits on raw bytes; an empty separator yields a flat list. At `MAX_DEPTH`
/// parts the rest of the key, separators included, becomes the last part.
fn split<'a>(key: &'a [u8], sep: &[u8]) -> Vec<&'a [u8]> {
    if sep.is_empty() {
        return vec![key];
    }
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while parts.len() + 1 < MAX_DEPTH && i + sep.len() <= key.len() {
        if &key[i..i + sep.len()] == sep {
            parts.push(&key[start..i]);
            i += sep.len();
            start = i;
        } else {
            i += 1;
        }
    }
    parts.push(&key[start..]);
    parts
}

pub fn group(keys: &[EntryKey], sep: &str) -> Vec<KeyNode> {
    let mut root = Builder::default();
    for key in keys {
        let mut node = &mut root;
        for part in split(&key.0, sep.as_bytes()) {
            node = node.children.entry(part.to_vec()).or_default();
        }
        node.key = Some(key.clone());
    }
    Builder::into_nodes(root.children)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn keys(raw: &[&str]) -> Vec<EntryKey> {
        raw.iter()
            .map(|k| EntryKey(k.as_bytes().to_vec()))
            .collect()
    }

    fn labels(nodes: &[KeyNode]) -> Vec<String> {
        nodes.iter().map(|n| n.label().into_owned()).collect()
    }

    fn depth(nodes: &[KeyNode]) -> usize {
        let mut d = 0;
        let mut level: Vec<&KeyNode> = nodes.iter().collect();
        while !level.is_empty() {
            d += 1;
            level = level.iter().flat_map(|n| n.children.iter()).collect();
        }
        d
    }

    #[test]
    fn deep_key_is_capped_at_max_depth() {
        // Arrange
        let raw = vec![b':'; 100_000];
        let key = EntryKey(raw);

        // Act
        let tree = group(std::slice::from_ref(&key), ":");

        // Assert
        assert!(depth(&tree) <= MAX_DEPTH);
        let mut node = &tree[0];
        while let Some(child) = node.children.first() {
            node = child;
        }
        assert_eq!(node.key, Some(key));
    }

    #[test]
    fn groups_keys_by_separator_prefix() {
        // Arrange
        let ks = keys(&["user:1", "user:2", "order:9"]);

        // Act
        let tree = group(&ks, ":");

        // Assert
        assert_eq!(labels(&tree), ["order", "user"]);
        assert_eq!(labels(&tree[1].children), ["1", "2"]);
        assert_eq!(tree[1].children[0].key, Some(EntryKey(b"user:1".to_vec())));
        assert!(tree[1].key.is_none());
    }

    #[test]
    fn custom_separator_groups_by_it() {
        // Arrange
        let ks = keys(&["a/b", "a/c"]);

        // Act
        let by_slash = group(&ks, "/");
        let by_colon = group(&ks, ":");

        // Assert
        assert_eq!(labels(&by_slash), ["a"]);
        assert_eq!(labels(&by_slash[0].children), ["b", "c"]);
        assert_eq!(labels(&by_colon), ["a/b", "a/c"]);
    }

    #[test]
    fn key_without_separator_is_leaf() {
        // Arrange
        let ks = keys(&["plain"]);

        // Act
        let tree = group(&ks, ":");

        // Assert
        assert_eq!(tree.len(), 1);
        assert!(tree[0].children.is_empty());
        assert_eq!(tree[0].key, Some(EntryKey(b"plain".to_vec())));
    }

    #[test]
    fn non_utf8_key_keeps_raw_bytes() {
        // Arrange
        let raw = EntryKey(vec![b'k', b':', 0xff, 0xfe]);

        // Act
        let tree = group(std::slice::from_ref(&raw), ":");

        // Assert
        assert_eq!(tree[0].children[0].segment, vec![0xff, 0xfe]);
        assert_eq!(tree[0].children[0].key, Some(raw));
        assert_eq!(tree[0].children[0].label(), "\u{fffd}\u{fffd}");
    }

    #[test]
    fn empty_separator_keeps_keys_flat() {
        // Arrange
        let ks = keys(&["a:b", "c"]);

        // Act
        let tree = group(&ks, "");

        // Assert
        assert_eq!(labels(&tree), ["a:b", "c"]);
        assert!(tree.iter().all(|n| n.children.is_empty()));
    }

    #[test]
    fn duplicate_keys_are_absorbed() {
        // Arrange
        let ks = keys(&["a:b", "a:b"]);

        // Act
        let tree = group(&ks, ":");

        // Assert
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].children.len(), 1);
    }

    #[test]
    fn node_can_be_key_and_prefix() {
        // Arrange
        let ks = keys(&["a", "a:b"]);

        // Act
        let tree = group(&ks, ":");

        // Assert
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].key, Some(EntryKey(b"a".to_vec())));
        assert_eq!(labels(&tree[0].children), ["b"]);
    }
}
