// ABOUTME: Deterministic, context-aware symbol catalog used before semantic inference.
// ABOUTME: Preserves ambiguous aliases and indexes lexical candidates by character trigrams.

use crate::module_index::ModuleIndex;
use codegraph_core::{CodeNode, EdgeRelationship, EdgeType, NodeId, NodeType};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone)]
struct Symbol {
    id: NodeId,
    file: String,
    qualified: String,
}

struct LexicalCandidate {
    alias: String,
    normalized: String,
    #[cfg(test)]
    gram_count: usize,
    characters: usize,
}

pub(crate) struct SymbolCatalog {
    aliases: BTreeMap<String, Vec<NodeId>>,
    symbols: HashMap<NodeId, Symbol>,
    lexical: Vec<LexicalCandidate>,
    trigrams: HashMap<String, Vec<usize>>,
    /// File node of each file path.
    file_nodes: HashMap<String, NodeId>,
    /// Module names of the indexed files.
    modules: ModuleIndex,
    /// Files each file imports, as mapped by `link_imports`.
    imported_files: HashMap<String, HashSet<String>>,
}

fn is_bare(name: &str) -> bool {
    !(name.contains("::") || name.contains('.') || name.contains('/'))
}

pub(crate) fn aliases(node: &CodeNode) -> BTreeSet<String> {
    let name = node.name.to_string();
    let mut names = BTreeSet::from([name.clone(), format!("{}::{name}", node.location.file_path)]);
    if let Some(qualified) = node.metadata.attributes.get("qualified_name") {
        names.insert(qualified.clone());
    }
    // The extractor's own qualified name, from before the CRG reshape.
    if let Some(original) = node
        .metadata
        .attributes
        .get(codegraph_parser::languages::crg_shape::PRE_SHAPE_QN_ATTR)
    {
        names.insert(original.clone());
    }
    if let Some(kind) = &node.node_type {
        names.insert(format!("{kind:?}::{name}"));
    }
    if let Some(short) = name.rsplit("::").next() {
        names.insert(short.to_string());
    }
    for attribute in ["method_of", "implements_trait"] {
        if let Some(owner) = node.metadata.attributes.get(attribute) {
            names.insert(format!("{owner}::{name}"));
        }
    }
    names
}

fn trigrams(name: &str) -> BTreeSet<String> {
    let chars: Vec<_> = name.chars().collect();
    if chars.len() < 3 {
        return BTreeSet::from([name.to_string()]);
    }
    chars
        .windows(3)
        .map(|window| window.iter().collect())
        .collect()
}

impl SymbolCatalog {
    pub(crate) fn new(nodes: &[CodeNode]) -> Self {
        let mut catalog = Self {
            aliases: BTreeMap::new(),
            symbols: HashMap::new(),
            lexical: Vec::new(),
            trigrams: HashMap::new(),
            file_nodes: HashMap::new(),
            modules: ModuleIndex::new([]),
            imported_files: HashMap::new(),
        };
        for node in nodes {
            if node.node_type == Some(NodeType::Other("File".into())) {
                catalog
                    .file_nodes
                    .insert(node.location.file_path.clone(), node.id);
            }
            catalog.symbols.insert(
                node.id,
                Symbol {
                    id: node.id,
                    file: node.location.file_path.clone(),
                    qualified: node
                        .metadata
                        .attributes
                        .get("qualified_name")
                        .cloned()
                        .unwrap_or_else(|| node.name.to_string()),
                },
            );
            for alias in aliases(node) {
                let ids = catalog.aliases.entry(alias).or_default();
                if !ids.contains(&node.id) {
                    ids.push(node.id);
                }
            }
        }
        catalog.modules = ModuleIndex::new(catalog.file_nodes.keys().map(String::as_str));
        for ids in catalog.aliases.values_mut() {
            ids.sort();
        }
        for alias in catalog.aliases.keys() {
            let normalized = alias.to_lowercase();
            let index = catalog.lexical.len();
            let grams = trigrams(&normalized);
            for gram in &grams {
                catalog
                    .trigrams
                    .entry(gram.clone())
                    .or_default()
                    .push(index);
            }
            catalog.lexical.push(LexicalCandidate {
                alias: alias.clone(),
                characters: normalized.chars().count(),
                normalized,
                #[cfg(test)]
                gram_count: grams.len(),
            });
        }
        catalog
    }

    /// Legacy consumers receive only aliases with exactly one target.
    pub(crate) fn unique_aliases(&self) -> HashMap<String, NodeId> {
        self.aliases
            .iter()
            .filter(|(_, ids)| ids.len() == 1)
            .map(|(alias, ids)| (alias.clone(), ids[0]))
            .collect()
    }

    pub(crate) fn ambiguous(&self, alias: &str) -> bool {
        self.aliases.get(alias).is_some_and(|ids| ids.len() > 1)
    }

    #[cfg(test)]
    pub(crate) fn semantic_candidates(&self, target: &str, limit: usize) -> Vec<String> {
        let target = target.to_lowercase();
        let grams = trigrams(&target);
        let target_len = target.chars().count().max(1) as f32;
        self.candidate_indices(&grams, limit)
            .into_iter()
            .filter_map(|(index, common)| {
                let candidate = &self.lexical[index];
                if self.aliases[&candidate.alias].len() != 1 {
                    return None;
                }
                let name_len = candidate.characters.max(1) as f32;
                if (target_len / name_len).min(name_len / target_len) < 0.5 {
                    return None;
                }
                let union = grams.len() + candidate.gram_count - common;
                (common as f32 / union.max(1) as f32 >= 0.2).then(|| candidate.alias.clone())
            })
            .collect()
    }

    fn choose(&self, ids: &[NodeId], from: NodeId) -> Option<NodeId> {
        if ids.len() == 1 {
            return ids.first().copied();
        }
        let source = self.symbols.get(&from)?;
        let local: Vec<_> = ids
            .iter()
            .filter_map(|id| self.symbols.get(id))
            .filter(|symbol| symbol.file == source.file)
            .collect();
        if local.len() == 1 {
            return Some(local[0].id);
        }
        let source_scope = source.qualified.rsplit_once("::").map(|(scope, _)| scope)?;
        let scoped: Vec<_> = local
            .iter()
            .filter(|symbol| {
                symbol.qualified.rsplit_once("::").map(|(scope, _)| scope) == Some(source_scope)
            })
            .collect();
        (scoped.len() == 1).then(|| scoped[0].id)
    }

    pub(crate) fn exact(&self, edge: &EdgeRelationship, variants: &[String]) -> Option<NodeId> {
        if let Some(id) = edge
            .metadata
            .get("target_node_id")
            .and_then(|id| NodeId::parse_str(id).ok())
            && self.symbols.contains_key(&id)
        {
            return Some(id);
        }
        for alias in std::iter::once(edge.to.as_str()).chain(variants.iter().map(String::as_str)) {
            if let Some(ids) = self.aliases.get(alias)
                && let Some(id) =
                    self.choose(&self.reachable(&edge.to, alias, ids, edge.from), edge.from)
            {
                return Some(id);
            }
        }
        None
    }

    /// Records which indexed files each file's `Imports` edges name, so that a bare name can
    /// bind into an imported file (`ModuleIndex` maps the import text to the file).
    pub(crate) fn link_imports(&mut self, edges: &[EdgeRelationship]) {
        for edge in edges.iter().filter(|e| e.edge_type == EdgeType::Imports) {
            let Some(importer) = self.symbols.get(&edge.from) else {
                continue;
            };
            if let Some(file) = self.modules.resolve(&importer.file, &edge.to) {
                self.imported_files
                    .entry(importer.file.clone())
                    .or_default()
                    .insert(file.to_string());
            }
        }
    }

    /// The File node an `Imports` edge names, when its text maps to one indexed file.
    pub(crate) fn import_target(&self, edge: &EdgeRelationship) -> Option<NodeId> {
        if edge.edge_type != EdgeType::Imports {
            return None;
        }
        let importer = &self.symbols.get(&edge.from)?.file;
        let file = self.modules.resolve(importer, &edge.to)?;
        self.file_nodes.get(file).copied()
    }

    /// Candidates `alias` may bind from `from` for the edge target `target`. Like
    /// code-review-graph, a bare name (no `::`, `.` or `/` in the target or the alias that
    /// matched it) binds only within the caller's file or a file it imports; qualified
    /// targets matched by qualified aliases bind anywhere.
    fn reachable(&self, target: &str, alias: &str, ids: &[NodeId], from: NodeId) -> Vec<NodeId> {
        if !is_bare(target) && !is_bare(alias) {
            return ids.to_vec();
        }
        let Some(source) = self.symbols.get(&from) else {
            return Vec::new();
        };
        let imported = self.imported_files.get(&source.file);
        ids.iter()
            .copied()
            .filter(|id| {
                self.symbols.get(id).is_some_and(|s| {
                    s.file == source.file || imported.is_some_and(|files| files.contains(&s.file))
                })
            })
            .collect()
    }

    /// A deterministic shortlist; a zero limit retains every overlapping candidate.
    fn candidate_indices(&self, grams: &BTreeSet<String>, limit: usize) -> Vec<(usize, usize)> {
        let mut counts = HashMap::<usize, usize>::new();
        for gram in grams {
            if let Some(indices) = self.trigrams.get(gram) {
                for index in indices {
                    *counts.entry(*index).or_default() += 1;
                }
            }
        }
        let mut ranked: Vec<_> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked
            .into_iter()
            .take(if limit == 0 { usize::MAX } else { limit })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn candidates(&self, target: &str, limit: usize) -> Vec<String> {
        self.candidate_indices(&trigrams(&target.to_lowercase()), limit)
            .into_iter()
            .map(|(index, _)| self.lexical[index].alias.clone())
            .collect()
    }

    pub(crate) fn fuzzy(&self, edge: &EdgeRelationship) -> Option<(NodeId, f64)> {
        let target = edge.to.to_lowercase();
        let target_length = target.chars().count();
        let scorer = rapidfuzz::distance::levenshtein::BatchComparator::new(target.chars());
        let mut best = 0.85f64;
        let mut matches = HashSet::new();
        for (index, _) in self.candidate_indices(&trigrams(&target), 0) {
            let candidate = &self.lexical[index];
            // Edit-distance similarity cannot exceed the ratio of the shorter length.
            if (target_length.min(candidate.characters) as f64)
                / (target_length.max(candidate.characters).max(1) as f64)
                + f64::EPSILON
                < best
            {
                continue;
            }
            let Some(score) = scorer.normalized_similarity_with_args(
                candidate.normalized.chars(),
                &rapidfuzz::distance::levenshtein::Args::default().score_cutoff(best),
            ) else {
                continue;
            };
            if score < best {
                continue;
            }
            let reachable = self.reachable(
                &edge.to,
                &candidate.alias,
                &self.aliases[&candidate.alias],
                edge.from,
            );
            let Some(id) = self.choose(&reachable, edge.from) else {
                continue;
            };
            if score > best {
                best = score;
                matches.clear();
            }
            matches.insert(id);
        }
        (matches.len() == 1).then(|| (*matches.iter().next().unwrap(), best))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codegraph_core::{EdgeType, Language, Location, NodeType};

    fn node(file: &str, name: &str) -> CodeNode {
        let mut node = CodeNode::new(
            name.to_string(),
            Some(NodeType::Function),
            Some(Language::Rust),
            Location {
                file_path: file.into(),
                line: 1,
                column: 1,
                end_line: None,
                end_column: None,
            },
        );
        node.set_deterministic_id("test");
        node
    }
    fn edge(from: NodeId, to: &str) -> EdgeRelationship {
        EdgeRelationship {
            from,
            to: to.into(),
            edge_type: EdgeType::Calls,
            metadata: Default::default(),
            span: None,
        }
    }

    #[test]
    fn pre_shape_qualified_name_stays_an_alias() {
        let mut method = node("m.rs", "new");
        method
            .metadata
            .attributes
            .insert("qualified_name".into(), "m.rs::A.new".into());
        method.metadata.attributes.insert(
            codegraph_parser::languages::crg_shape::PRE_SHAPE_QN_ATTR.into(),
            "m.rs::impl<A>::new".into(),
        );
        let names = aliases(&method);
        assert!(names.contains("m.rs::A.new") && names.contains("m.rs::impl<A>::new"));
    }

    #[test]
    fn ambiguous_names_are_scoped_and_direct_definitions_remain_authoritative() {
        let a = node("a.rs", "duplicate");
        let b = node("b.rs", "duplicate");
        let caller = node("b.rs", "caller");
        for nodes in [
            vec![a.clone(), b.clone(), caller.clone()],
            vec![caller.clone(), b.clone(), a.clone()],
        ] {
            let catalog = SymbolCatalog::new(&nodes);
            assert!(!catalog.unique_aliases().contains_key("duplicate"));
            assert_eq!(
                catalog.exact(&edge(caller.id, "duplicate"), &[]),
                Some(b.id)
            );
            assert_eq!(
                catalog.exact(&edge(NodeId::new_v4(), "duplicate"), &[]),
                None
            );
            let mut direct = edge(caller.id, "duplicate");
            direct
                .metadata
                .insert("target_node_id".into(), a.id.to_string());
            assert_eq!(catalog.exact(&direct, &[]), Some(a.id));
        }
    }

    #[test]
    fn bare_names_bind_only_within_the_callers_file() {
        let helper = node("b.rs", "helper");
        let local = node("a.rs", "local");
        let caller = node("a.rs", "caller");
        let catalog = SymbolCatalog::new(&[helper.clone(), local.clone(), caller.clone()]);
        assert_eq!(catalog.exact(&edge(caller.id, "helper"), &[]), None);
        assert_eq!(catalog.fuzzy(&edge(caller.id, "helper")), None);
        assert_eq!(
            catalog.exact(&edge(caller.id, "local"), &[]),
            Some(local.id)
        );
        assert_eq!(
            catalog.exact(&edge(caller.id, "b.rs::helper"), &[]),
            Some(helper.id)
        );
    }

    #[test]
    fn imports_let_bare_names_bind_into_the_imported_file() {
        let mut file_a = node("a.py", "a.py");
        file_a.node_type = Some(NodeType::Other("File".into()));
        let mut file_b = node("b.py", "b.py");
        file_b.node_type = Some(NodeType::Other("File".into()));
        let helper = node("b.py", "helper");
        let other = node("c.py", "other");
        let caller = node("a.py", "caller");
        let mut catalog = SymbolCatalog::new(&[
            file_a.clone(),
            file_b.clone(),
            helper.clone(),
            other.clone(),
            caller.clone(),
        ]);
        let mut import = edge(file_a.id, "b");
        import.edge_type = EdgeType::Imports;
        assert_eq!(catalog.import_target(&import), Some(file_b.id));
        assert_eq!(catalog.exact(&edge(caller.id, "helper"), &[]), None);
        catalog.link_imports(&[import]);
        assert_eq!(
            catalog.exact(&edge(caller.id, "helper"), &[]),
            Some(helper.id)
        );
        assert_eq!(catalog.exact(&edge(caller.id, "other"), &[]), None);
    }

    #[test]
    fn a_bare_target_never_reaches_another_file_through_a_qualified_alias() {
        let mut target = node("b.rs", "longhelpername");
        target
            .metadata
            .attributes
            .insert("method_of".into(), "Owner".into());
        let caller = node("a.rs", "caller");
        let catalog = SymbolCatalog::new(&[target.clone(), caller.clone()]);
        // Close to the qualified alias `Owner::longhelpername`, but the call text is bare.
        assert_eq!(catalog.fuzzy(&edge(caller.id, "Ownerlonghelpername")), None);
        assert_eq!(
            catalog
                .fuzzy(&edge(caller.id, "Owner::longhelpernam"))
                .map(|(id, _)| id),
            Some(target.id)
        );
    }

    #[test]
    fn lexical_candidates_are_deterministic_and_unicode_safe() {
        let target = node("a.rs", "calculate_result");
        let catalog = SymbolCatalog::new(&[target.clone(), node("b.rs", "unrelated")]);
        assert_eq!(
            catalog
                .fuzzy(&edge(target.id, "calculate_reslt"))
                .map(|(id, _)| id),
            Some(target.id)
        );
        assert!(!catalog.candidates("calculate", 2).is_empty());
        assert!(!catalog.semantic_candidates("calculate_reslt", 0).is_empty());
        assert!(catalog.candidates("🦀", 0).is_empty());
    }
}
