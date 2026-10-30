//! Pure filtering and grouping of the saved-connection list shown on the home screen.
use crate::storage::config::{ConnectionConfig, Label};

/// Name of the group holding connections without a label.
pub const UNGROUPED: &str = "Ungrouped";

/// Case-insensitive text search over name, host and label, ANDed with the exact label filter.
pub fn filter_connections<'a>(
    conns: &'a [ConnectionConfig],
    query: &str,
    label: Option<&str>,
) -> Vec<&'a ConnectionConfig> {
    let q = query.trim().to_lowercase();
    conns
        .iter()
        .filter(|c| label.is_none_or(|l| c.label.as_deref() == Some(l)))
        .filter(|c| {
            q.is_empty()
                || [
                    Some(c.name.as_str()),
                    Some(c.host.as_str()),
                    c.label.as_deref(),
                ]
                .into_iter()
                .flatten()
                .any(|f| f.to_lowercase().contains(&q))
        })
        .collect()
}

/// One folder per label: catalog order, then labels missing from the catalog, then `None`
/// (unlabeled) last. Empty groups are omitted.
pub fn group_by_label<'a>(
    conns: &[&'a ConnectionConfig],
    cat: &[Label],
) -> Vec<(Option<String>, Vec<&'a ConnectionConfig>)> {
    let mut names: Vec<&str> = cat.iter().map(|l| l.name.as_str()).collect();
    for n in conns.iter().filter_map(|c| c.label.as_deref()) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    let named = names.into_iter().map(Some);
    named
        .chain([None])
        .map(|n| {
            let items: Vec<_> = conns
                .iter()
                .copied()
                .filter(|c| c.label.as_deref() == n)
                .collect();
            (n.map(String::from), items)
        })
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::workspace::tests::state;

    fn conns(spec: &[(&str, &str, Option<&str>)]) -> Vec<ConnectionConfig> {
        let mut s = state(&spec.iter().map(|x| x.0).collect::<Vec<_>>());
        for (c, (_, host, label)) in s.config.connections.iter_mut().zip(spec) {
            c.host = (*host).into();
            c.label = label.map(Into::into);
        }
        s.config.connections
    }

    fn cat(names: &[&str]) -> Vec<Label> {
        names
            .iter()
            .map(|n| Label {
                name: (*n).into(),
                color: [1, 2, 3],
            })
            .collect()
    }

    fn names<'a>(v: &[&'a ConnectionConfig]) -> Vec<&'a str> {
        v.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn filter_matches_name_host_label_case_insensitive() {
        // Arrange
        let c = conns(&[
            ("Alpha", "h1", None),
            ("b", "DB.Example.com", None),
            ("c", "h3", Some("Staging")),
        ]);

        // Act & Assert
        assert_eq!(names(&filter_connections(&c, "ALPH", None)), ["Alpha"]);
        assert_eq!(names(&filter_connections(&c, "example", None)), ["b"]);
        assert_eq!(names(&filter_connections(&c, "stag", None)), ["c"]);
    }

    #[test]
    fn filter_and_with_label_filter() {
        // Arrange
        let c = conns(&[("a1", "h", Some("dev")), ("a2", "h", Some("prod"))]);

        // Act
        let r = filter_connections(&c, "a", Some("dev"));

        // Assert
        assert_eq!(names(&r), ["a1"]);
    }

    #[test]
    fn empty_query_returns_all() {
        // Arrange
        let c = conns(&[("a", "h", None), ("b", "h", Some("x"))]);

        // Act
        let r = filter_connections(&c, "  ", None);

        // Assert
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn group_by_label_follows_catalog_order_ungrouped_last() {
        // Arrange
        let c = conns(&[
            ("n", "h", None),
            ("p", "h", Some("prod")),
            ("d", "h", Some("dev")),
        ]);
        let all: Vec<_> = c.iter().collect();

        // Act
        let g = group_by_label(&all, &cat(&["dev", "prod"]));

        // Assert
        let keys: Vec<_> = g.iter().map(|(k, _)| k.as_deref()).collect();
        assert_eq!(keys, [Some("dev"), Some("prod"), None]);
    }

    #[test]
    fn orphan_label_gets_own_group() {
        // Arrange
        let c = conns(&[("o", "h", Some("gone")), ("d", "h", Some("dev"))]);
        let all: Vec<_> = c.iter().collect();

        // Act
        let g = group_by_label(&all, &cat(&["dev"]));

        // Assert
        let keys: Vec<_> = g.iter().map(|(k, _)| k.as_deref()).collect();
        assert_eq!(keys, [Some("dev"), Some("gone")]);
    }

    #[test]
    fn empty_groups_omitted() {
        // Arrange
        let c = conns(&[("d", "h", Some("dev"))]);
        let all: Vec<_> = c.iter().collect();

        // Act
        let g = group_by_label(&all, &cat(&["dev", "prod"]));

        // Assert
        assert_eq!(g.len(), 1);
    }
}
