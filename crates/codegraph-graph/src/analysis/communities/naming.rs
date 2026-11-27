// ABOUTME: Names communities from their members and keeps the names unique.
// ABOUTME: Literal port of the naming rules in communities.py (code-review-graph).

use super::Community;
use crate::analysis::{AnalysisGraph, AnalysisNode};
use std::collections::{HashMap, HashSet};

const SLUG_MAX_LEN: usize = 30;

const COMMON_WORDS: &[&str] = &[
    "get", "set", "self", "init", "new", "create", "update", "delete", "add", "remove", "make",
    "build", "from", "to", "for", "with", "the", "and", "test", "main", "run", "do", "is", "has",
    "on", "of", "in", "at", "by", "my", "this", "that", "all", "none", "should", "when", "then",
    "given", "return", "returns", "raise", "raises", "expect", "expected", "assert", "tests", "be",
    "it", "if", "not",
];

fn is_test_node(n: &AnalysisNode) -> bool {
    n.kind == "Test" || n.is_test
}

/// Counts in first-seen order, then stable-sorted by count, highest first (`Counter.most_common`).
fn most_common<'a>(items: impl IntoIterator<Item = &'a str>) -> Vec<(&'a str, usize)> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for item in items {
        match index.get(item) {
            Some(&i) => counts[i].1 += 1,
            None => {
                index.insert(item, counts.len());
                counts.push((item, 1));
            }
        }
    }
    counts.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    counts
}

/// Splits camelCase and snake/kebab/dotted names into words.
pub fn split_name(name: &str) -> Vec<String> {
    let mut spaced = String::with_capacity(name.len() + 4);
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if prev.is_some_and(|p| p.is_ascii_lowercase()) && c.is_ascii_uppercase() {
            spaced.push('_');
        }
        spaced.push(c);
        prev = Some(c);
    }
    spaced
        .split(|c: char| c == '_' || c == '-' || c == '.' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Lowercase hyphenated slug, cut at a word boundary within 30 characters.
pub fn to_slug(s: &str) -> String {
    let normalized: String = {
        let mut out = String::new();
        let mut in_sep = false;
        for c in s.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c);
                in_sep = false;
            } else if !in_sep {
                out.push(' ');
                in_sep = true;
            }
        }
        out
    };
    let slug = split_name(&normalized)
        .iter()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join("-");
    if slug.len() <= SLUG_MAX_LEN {
        return slug;
    }
    match slug[..=SLUG_MAX_LEN].rfind('-') {
        Some(boundary) if boundary > 0 => slug[..boundary].to_string(),
        _ => slug[..SLUG_MAX_LEN].to_string(),
    }
}

fn naming_members<'a>(members: &[&'a AnalysisNode]) -> Vec<&'a AnalysisNode> {
    let production: Vec<&AnalysisNode> = members
        .iter()
        .copied()
        .filter(|m| !is_test_node(m))
        .collect();
    if production.is_empty() {
        members.to_vec()
    } else {
        production
    }
}

fn file_prefix(paths: &[&str]) -> String {
    let parts: Vec<String> = paths
        .iter()
        .map(|p| {
            let normalized = p.replace('\\', "/");
            let segments: Vec<&str> = normalized.split('/').collect();
            if segments.len() >= 2 {
                segments[segments.len() - 2].to_string()
            } else {
                let last = segments[0];
                last.rsplit_once('.')
                    .map_or(last, |(stem, _)| stem)
                    .to_string()
            }
        })
        .collect();
    most_common(parts.iter().map(String::as_str))
        .first()
        .map(|(top, _)| to_slug(top))
        .unwrap_or_default()
}

fn keywords(members: &[&AnalysisNode]) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for m in members {
        if matches!(m.kind.as_str(), "Function" | "Class" | "Test" | "Type") {
            for w in split_name(&m.name) {
                let lower = w.to_lowercase();
                if lower.chars().count() > 1 && !COMMON_WORDS.contains(&lower.as_str()) {
                    words.push(lower);
                }
            }
        }
    }
    most_common(words.iter().map(String::as_str))
        .into_iter()
        .take(5)
        .map(|(w, _)| w.to_string())
        .collect()
}

/// Name for a set of member nodes: dominant class, else `prefix-keyword`, `prefix`, `keyword`.
pub fn generate_name(members: &[&AnalysisNode]) -> String {
    if members.is_empty() {
        return "empty".to_string();
    }
    let members = naming_members(members);
    let paths: Vec<&str> = members.iter().map(|m| m.file_path.as_str()).collect();
    let prefix = file_prefix(&paths);
    let classes: Vec<&str> = members
        .iter()
        .filter(|m| m.kind == "Class")
        .map(|m| m.name.as_str())
        .collect();
    if let Some(&(top, count)) = most_common(classes).first()
        && count as f64 > members.len() as f64 * 0.4
    {
        return if prefix.is_empty() {
            to_slug(top)
        } else {
            format!("{prefix}-{}", to_slug(top))
        };
    }
    let keyword = keywords(&members).into_iter().next().unwrap_or_default();
    match (prefix.is_empty(), keyword.is_empty()) {
        (false, false) => format!("{prefix}-{keyword}"),
        (false, true) => prefix,
        (true, false) => keyword,
        (true, true) => "cluster".to_string(),
    }
}

fn members_of<'a>(g: &'a AnalysisGraph, comm: &Community) -> Vec<&'a AnalysisNode> {
    comm.members
        .iter()
        .filter_map(|qn| g.index_of(qn).map(|i| &g.nodes[i]))
        .collect()
}

/// Names every community that has no name yet.
pub fn fill_names(g: &AnalysisGraph, comms: &mut [Community]) {
    for comm in comms.iter_mut().filter(|c| c.name.is_empty()) {
        comm.name = generate_name(&members_of(g, comm));
    }
}

/// Makes duplicate names unique: the largest keeps the name; the others get a keyword suffix not
/// already in the name, else `-2`, `-3`, ...
pub fn dedupe_names(g: &AnalysisGraph, comms: &mut [Community]) {
    let mut by_name: Vec<(String, Vec<usize>)> = Vec::new();
    let mut slot: HashMap<String, usize> = HashMap::new();
    for (pos, c) in comms.iter().enumerate() {
        let i = *slot.entry(c.name.clone()).or_insert_with(|| {
            by_name.push((c.name.clone(), Vec::new()));
            by_name.len() - 1
        });
        by_name[i].1.push(pos);
    }
    let mut taken: HashSet<String> = comms
        .iter()
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .collect();
    for (base, mut positions) in by_name {
        if base.is_empty() || positions.len() <= 1 {
            continue;
        }
        positions.sort_by(|&a, &b| comms[b].size.cmp(&comms[a].size).then(a.cmp(&b)));
        let base_words: HashSet<&str> = base.split('-').collect();
        for &pos in &positions[1..] {
            let members = members_of(g, &comms[pos]);
            let mut chosen = String::new();
            for keyword in keywords(&naming_members(&members)) {
                let suffix = to_slug(&keyword);
                if suffix.is_empty() || base_words.contains(suffix.as_str()) {
                    continue;
                }
                let candidate = format!("{base}-{suffix}");
                if !taken.contains(&candidate) {
                    chosen = candidate;
                    break;
                }
            }
            if chosen.is_empty() {
                let mut n = 2;
                chosen = format!("{base}-{n}");
                while taken.contains(&chosen) {
                    n += 1;
                    chosen = format!("{base}-{n}");
                }
            }
            comms[pos].name = chosen.clone();
            taken.insert(chosen);
        }
    }
}

/// Generates names for unnamed communities, then makes all names unique.
pub fn name_communities(g: &AnalysisGraph, comms: &mut [Community]) {
    fill_names(g, comms);
    dedupe_names(g, comms);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::AnalysisGraph;
    use crate::analysis::communities::Community;
    use crate::sqlite_store::SqliteGraphStore;

    fn graph(nodes: &[(&str, &str, &str, &str)]) -> AnalysisGraph {
        // (kind, qualified_name, name, file_path)
        let dir = tempfile::tempdir().unwrap();
        let _store = SqliteGraphStore::open(dir.path()).unwrap();
        let conn =
            rusqlite::Connection::open(dir.path().join(".code-review-graph/graph.db")).unwrap();
        for (kind, qn, name, file) in nodes {
            conn.execute(
                "INSERT INTO nodes (kind, name, qualified_name, file_path, is_test, extra, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, '{}', 0)",
                rusqlite::params![kind, name, qn, file, (*kind == "Test") as i64],
            )
            .unwrap();
        }
        AnalysisGraph::load(&conn).unwrap()
    }

    fn community(members: &[&str]) -> Community {
        Community {
            name: String::new(),
            level: 0,
            cohesion: 0.0,
            size: members.len(),
            dominant_language: String::new(),
            description: String::new(),
            members: members.iter().map(|m| m.to_string()).collect(),
            detector: "leiden",
        }
    }

    #[test]
    fn split_name_breaks_camel_and_snake_case() {
        assert_eq!(split_name("parseHTTPRequest"), ["parse", "HTTPRequest"]);
        assert_eq!(
            split_name("load_user-data.v2"),
            ["load", "user", "data", "v2"]
        );
    }

    #[test]
    fn to_slug_cuts_at_a_word_boundary() {
        assert_eq!(to_slug("Cart Item"), "cart-item");
        let long = "alpha beta gamma delta epsilon zeta eta theta";
        assert_eq!(to_slug(long), "alpha-beta-gamma-delta-epsilon");
        assert_eq!(to_slug(&"x".repeat(40)), "x".repeat(30));
    }

    #[test]
    fn dominant_class_names_community() {
        let g = graph(&[
            ("Class", "shop/cart.py::Cart", "Cart", "shop/cart.py"),
            ("Class", "shop/cart2.py::Cart", "Cart", "shop/cart2.py"),
            ("Class", "shop/cart3.py::Cart", "Cart", "shop/cart3.py"),
            ("Function", "shop/f.py::total", "total", "shop/f.py"),
            ("Function", "shop/f.py::tax", "tax", "shop/f.py"),
        ]);
        let mut comms = vec![community(
            &g.nodes
                .iter()
                .map(|n| n.qualified.as_str())
                .collect::<Vec<_>>(),
        )];
        name_communities(&g, &mut comms);
        assert_eq!(comms[0].name, "shop-cart");
    }

    #[test]
    fn keyword_with_prefix() {
        let g = graph(&[
            (
                "Function",
                "users/repo.py::load_user",
                "load_user",
                "users/repo.py",
            ),
            (
                "Function",
                "users/repo.py::save_user",
                "save_user",
                "users/repo.py",
            ),
            (
                "Function",
                "users/repo.py::find_user",
                "find_user",
                "users/repo.py",
            ),
        ]);
        let mut comms = vec![community(&[
            "users/repo.py::load_user",
            "users/repo.py::save_user",
            "users/repo.py::find_user",
        ])];
        name_communities(&g, &mut comms);
        assert_eq!(comms[0].name, "users-user");
    }

    #[test]
    fn all_tests_still_named() {
        let g = graph(&[
            ("Test", "tests/t.py::test_login", "test_login", "tests/t.py"),
            (
                "Test",
                "tests/t.py::test_logout",
                "test_logout",
                "tests/t.py",
            ),
        ]);
        let mut comms = vec![community(&[
            "tests/t.py::test_login",
            "tests/t.py::test_logout",
        ])];
        name_communities(&g, &mut comms);
        assert_eq!(comms[0].name, "tests-login");
    }

    #[test]
    fn empty_and_unnameable_communities() {
        let g = graph(&[("Function", "f.py::x", "x", "f.py")]);
        let mut comms = vec![community(&[])];
        name_communities(&g, &mut comms);
        assert_eq!(comms[0].name, "empty");
        let mut one = vec![community(&["f.py::x"])];
        name_communities(&g, &mut one);
        // Parent directory missing, so the file stem is the prefix; "x" is a one-letter word.
        assert_eq!(one[0].name, "f");
    }

    fn duplicate_graph(second: [&str; 3]) -> (AnalysisGraph, Vec<Community>) {
        let mut rows: Vec<(String, String, String)> = ["a1", "a2", "a3", "a4"]
            .iter()
            .map(|n| {
                (
                    format!("app/a.py::user_{n}"),
                    format!("user_{n}"),
                    "app/a.py".to_string(),
                )
            })
            .collect();
        rows.extend(second.iter().map(|n| {
            (
                format!("app/b.py::{n}"),
                n.to_string(),
                "app/b.py".to_string(),
            )
        }));
        let spec: Vec<(&str, &str, &str, &str)> = rows
            .iter()
            .map(|(qn, n, f)| ("Function", qn.as_str(), n.as_str(), f.as_str()))
            .collect();
        let g = graph(&spec);
        let first: Vec<&str> = g.nodes[..4].iter().map(|n| n.qualified.as_str()).collect();
        let rest: Vec<&str> = g.nodes[4..].iter().map(|n| n.qualified.as_str()).collect();
        let comms = vec![community(&rest), community(&first)];
        (g, comms)
    }

    #[test]
    fn duplicates_get_keyword_suffix() {
        let (g, mut comms) = duplicate_graph(["user_alpha", "user_beta", "user_gamma"]);
        name_communities(&g, &mut comms);
        // The larger community keeps the base name; the smaller adds its first new keyword.
        assert_eq!(comms[1].name, "app-user");
        assert_eq!(comms[0].name, "app-user-alpha");
    }

    #[test]
    fn duplicates_fall_back_to_number() {
        let (g, mut comms) = duplicate_graph(["user_a", "user_b", "user_c"]);
        name_communities(&g, &mut comms);
        assert_eq!(comms[1].name, "app-user");
        assert_eq!(comms[0].name, "app-user-2");
    }
}
