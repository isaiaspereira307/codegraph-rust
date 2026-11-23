// ABOUTME: Response shaping for the community tools: member caps and the minimal architecture view.
// ABOUTME: Pure functions over GraphQuery community types so the limits are testable offline.

use codegraph_graph::graph_query::ArchitectureOverview;
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;

/// Most member entries a community may list, whatever the caller asks for.
pub const MAX_MEMBERS: usize = 25;

/// Cuts `members` and `member_details` to `max_members` (at most [`MAX_MEMBERS`]). When cut,
/// `{key}_total` keeps the real count and `members_truncated` is set; `size` is never touched.
pub fn cap_members(community: &mut JsonValue, max_members: usize) {
    let limit = max_members.clamp(1, MAX_MEMBERS);
    for key in ["members", "member_details"] {
        let Some(rows) = community.get_mut(key).and_then(JsonValue::as_array_mut) else {
            continue;
        };
        if rows.len() > limit {
            let total = rows.len();
            rows.truncate(limit);
            community[format!("{key}_total")] = json!(total);
            community["members_truncated"] = json!(true);
        }
    }
}

/// Communities without member lists, and one row per community pair with its edge count and the
/// three most common edge kinds (largest first; pairs by count, then by community ids).
pub fn minimal_overview(overview: &ArchitectureOverview) -> JsonValue {
    let communities: Vec<JsonValue> = overview
        .communities
        .iter()
        .map(|c| {
            json!({"id": c.id, "name": c.name, "size": c.size, "cohesion": c.cohesion,
                   "dominant_language": c.dominant_language})
        })
        .collect();
    let names: BTreeMap<i64, &str> = overview
        .communities
        .iter()
        .map(|c| (c.id, c.name.as_str()))
        .collect();
    let mut pairs: BTreeMap<(i64, i64), (usize, Vec<(String, usize)>)> = BTreeMap::new();
    for e in &overview.cross_community_edges {
        let key = (
            e.source_community.min(e.target_community),
            e.source_community.max(e.target_community),
        );
        let entry = pairs.entry(key).or_default();
        entry.0 += 1;
        match entry.1.iter_mut().find(|(k, _)| *k == e.edge_kind) {
            Some(kind) => kind.1 += 1,
            None => entry.1.push((e.edge_kind.clone(), 1)),
        }
    }
    let name_of = |id: i64| {
        names
            .get(&id)
            .map_or_else(|| format!("community-{id}"), |n| n.to_string())
    };
    let mut ordered: Vec<_> = pairs.into_iter().collect();
    ordered.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
    let cross: Vec<JsonValue> = ordered
        .into_iter()
        .map(|((a, b), (count, mut kinds))| {
            kinds.sort_by(|x, y| y.1.cmp(&x.1));
            json!({"source_community": name_of(a), "target_community": name_of(b),
                   "edge_count": count,
                   "top_kinds": kinds.into_iter().take(3).map(|(k, _)| k).collect::<Vec<_>>()})
        })
        .collect();
    json!({"communities": communities, "cross_community_edges": cross, "warnings": overview.warnings})
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_graph::graph_query::{ArchitectureOverview, CommunityRow, CrossCommunityEdge};
    use serde_json::json;

    fn community(id: i64, name: &str, members: usize) -> CommunityRow {
        CommunityRow {
            id,
            name: name.into(),
            level: 0,
            cohesion: 0.5,
            size: members as i64,
            dominant_language: "python".into(),
            description: "d".into(),
            members: (0..members).map(|i| format!("m{i}")).collect(),
        }
    }

    fn edge(a: i64, b: i64, kind: &str) -> CrossCommunityEdge {
        CrossCommunityEdge {
            source_community: a,
            target_community: b,
            edge_kind: kind.into(),
            source: "s".into(),
            target: "t".into(),
        }
    }

    #[test]
    fn list_communities_sorting_and_member_cap() {
        let mut v = serde_json::to_value(community(1, "big", 30)).unwrap();
        cap_members(&mut v, 10);
        assert_eq!(v["members"].as_array().unwrap().len(), 10);
        assert_eq!(v["members_total"], 30);
        assert_eq!(v["members_truncated"], true);
        assert_eq!(v["size"], 30);
        // The ceiling (25) wins over a larger request.
        let mut v = serde_json::to_value(community(1, "big", 30)).unwrap();
        cap_members(&mut v, 100);
        assert_eq!(v["members"].as_array().unwrap().len(), 25);
        // Small communities are left alone.
        let mut v = serde_json::to_value(community(2, "small", 3)).unwrap();
        cap_members(&mut v, 10);
        assert!(v.get("members_truncated").is_none());
        assert_eq!(v["members"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn minimal_overview_aggregates_pairs_with_top_kinds() {
        let overview = ArchitectureOverview {
            communities: vec![
                community(1, "a", 2),
                community(2, "b", 2),
                community(3, "c", 2),
            ],
            cross_community_edges: vec![
                edge(1, 2, "CALLS"),
                edge(2, 1, "CALLS"),
                edge(1, 2, "IMPORTS_FROM"),
                edge(1, 3, "CALLS"),
            ],
            warnings: vec!["w".into()],
        };
        let v = minimal_overview(&overview);
        assert_eq!(
            v["communities"][0],
            json!({"id": 1, "name": "a", "size": 2, "cohesion": 0.5, "dominant_language": "python"})
        );
        assert_eq!(
            v["cross_community_edges"],
            json!([
                {"source_community": "a", "target_community": "b", "edge_count": 3, "top_kinds": ["CALLS", "IMPORTS_FROM"]},
                {"source_community": "a", "target_community": "c", "edge_count": 1, "top_kinds": ["CALLS"]},
            ])
        );
        assert_eq!(v["warnings"], json!(["w"]));
    }
}
