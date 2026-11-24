// ABOUTME: Deterministic, context-aware symbol catalog used before semantic inference.
// ABOUTME: Preserves ambiguous aliases and indexes lexical candidates by character trigrams.

use crate::module_index::ModuleIndex;
use crate::rust_crates::RustCrates;
use codegraph_core::{CodeNode, EdgeRelationship, EdgeType, NodeId, NodeType};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone)]
struct Symbol {
    id: NodeId,
    file: String,
    qualified: String,
    /// Declared in a class or type (`parent_name`): a method, not a free function.
    has_parent: bool,
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
    /// Per Rust file, each `use` alias and the path it imports (`cfg` -> `crate::config`).
    rust_aliases: HashMap<String, HashMap<String, String>>,
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
    /// Rebuilds the module index so Rust `use` paths resolve through these Cargo layouts.
    pub(crate) fn set_rust_crates(&mut self, crates: RustCrates) {
        self.modules =
            ModuleIndex::new(self.file_nodes.keys().map(String::as_str)).with_rust_crates(crates);
    }

    /// Indexed `.rs` files, sorted: the importers `RustCrates::discover` reads manifests for.
    pub(crate) fn rust_files(&self) -> std::collections::BTreeSet<String> {
        self.file_nodes
            .keys()
            .filter(|file| file.ends_with(".rs"))
            .cloned()
            .collect()
    }

    pub(crate) fn new(nodes: &[CodeNode]) -> Self {
        let mut catalog = Self {
            aliases: BTreeMap::new(),
            symbols: HashMap::new(),
            lexical: Vec::new(),
            trigrams: HashMap::new(),
            file_nodes: HashMap::new(),
            modules: ModuleIndex::new([]),
            imported_files: HashMap::new(),
            rust_aliases: HashMap::new(),
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
                    has_parent: node.metadata.attributes.contains_key("parent_name"),
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

    /// Whether `target` is spelled `<indexed file>::<symbol>`, as extractors name a symbol of
    /// a file they know (code-review-graph's path-qualified endpoint).
    pub(crate) fn is_path_qualified(&self, target: &str) -> bool {
        target
            .split_once("::")
            .is_some_and(|(head, _)| self.file_nodes.contains_key(head))
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
        self.exact_where(edge, variants, |_| true)
    }

    /// `exact` without variants, skipping methods: what a call on a receiver of unknown type
    /// can still mean (code-review-graph `_receiver_is_untyped`).
    pub(crate) fn exact_free_function(&self, edge: &EdgeRelationship) -> Option<NodeId> {
        self.exact_where(edge, &[], |symbol| !symbol.has_parent)
    }

    fn exact_where(
        &self,
        edge: &EdgeRelationship,
        variants: &[String],
        keep: impl Fn(&Symbol) -> bool,
    ) -> Option<NodeId> {
        if let Some(id) = edge
            .metadata
            .get("target_node_id")
            .and_then(|id| NodeId::parse_str(id).ok())
            && self.symbols.get(&id).is_some_and(&keep)
        {
            return Some(id);
        }
        for alias in std::iter::once(edge.to.as_str()).chain(variants.iter().map(String::as_str)) {
            let Some(ids) = self.aliases.get(alias) else {
                continue;
            };
            let mut reachable = self.reachable(&edge.to, alias, ids, edge.from);
            reachable.retain(|id| self.symbols.get(id).is_some_and(&keep));
            // A target spelled as a symbol's exact qualified name (`f.java::User`) names that
            // symbol, not a member that merely shares the `file::name` alias (`User.User`).
            let named: Vec<_> = reachable
                .iter()
                .filter(|id| self.symbols.get(id).is_some_and(|s| s.qualified == alias))
                .collect();
            if let [id] = named.as_slice() {
                return Some(**id);
            }
            if let Some(id) = self.choose(&reachable, edge.from) {
                return Some(id);
            }
        }
        None
    }

    /// Records which indexed files each file's `Imports` edges name, so that a bare name can
    /// bind into an imported file (`ModuleIndex` maps the import text to the file), and the
    /// local alias of each Rust `use` path (`RUST_ALIAS_ATTR`; a glob import has none).
    pub(crate) fn link_imports(&mut self, edges: &[EdgeRelationship]) {
        for edge in edges.iter().filter(|e| e.edge_type == EdgeType::Imports) {
            let Some(importer) = self.symbols.get(&edge.from) else {
                continue;
            };
            if let Some(alias) = edge
                .metadata
                .get(codegraph_parser::languages::crg_shape::RUST_ALIAS_ATTR)
                .filter(|alias| !matches!(alias.as_str(), "" | "*" | "_"))
            {
                self.rust_aliases
                    .entry(importer.file.clone())
                    .or_default()
                    .insert(alias.clone(), edge.to.clone());
            }
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

    /// Like code-review-graph's `_python_from_import_targets`, drops the `M.x` candidates of
    /// `from M import x` (`PY_SUBMODULE_ATTR`) that name no file, and drops `M` itself when
    /// it names no file but one of its statement's candidates does (a namespace package).
    /// A statement is its importer plus its span.
    pub(crate) fn retain_python_imports(&self, edges: &mut Vec<EdgeRelationship>) {
        type Statement = (NodeId, Option<(u32, u32)>);
        let statement = |e: &EdgeRelationship| -> Statement {
            (e.from, e.span.as_ref().map(|s| (s.start_byte, s.end_byte)))
        };
        let candidate = |e: &EdgeRelationship| {
            e.edge_type == EdgeType::Imports
                && e.metadata
                    .contains_key(codegraph_parser::languages::crg_shape::PY_SUBMODULE_ATTR)
        };
        // Statements with candidates -> whether any candidate names a file.
        let mut any_file: HashMap<Statement, bool> = HashMap::new();
        for edge in edges.iter().filter(|e| candidate(e)) {
            *any_file.entry(statement(edge)).or_default() |= self.import_target(edge).is_some();
        }
        if any_file.is_empty() {
            return;
        }
        edges.retain(|edge| {
            let checked = candidate(edge)
                || (edge.edge_type == EdgeType::Imports
                    && any_file.get(&statement(edge)) == Some(&true));
            !checked || self.import_target(edge).is_some()
        });
    }

    /// Binds a Python call or reference carrying `PY_IMPORT_ATTR` (`"M|X"`): when `M.X` (or
    /// `M` for an empty `X`) is a module file, its top-level `<file>::<to>`; otherwise, in the
    /// file of `M`, `<file>::X.<to>` for a call on a receiver or `<file>::X` for a bare name
    /// (`to` is then the local alias). Never searches beyond those names.
    pub(crate) fn py_import_target(&self, edge: &EdgeRelationship) -> Option<NodeId> {
        let binding = edge
            .metadata
            .get(codegraph_parser::languages::crg_shape::PY_IMPORT_ATTR)?;
        let (module, symbol) = binding.split_once('|')?;
        let importer = &self.symbols.get(&edge.from)?.file;
        let dotted = match (symbol.is_empty(), module.ends_with('.')) {
            (true, _) => module.to_string(),
            (false, true) => format!("{module}{symbol}"),
            (false, false) => format!("{module}.{symbol}"),
        };
        if let Some(file) = self.modules.resolve(importer, &dotted) {
            if !symbol.is_empty() && !edge.metadata.contains_key("receiver") {
                // `from M import sub; sub()` / `run(sub)` names the module itself.
                return self.file_nodes.get(file).copied();
            }
            return self.qualified_symbol(&format!("{file}::{}", edge.to), true);
        }
        if symbol.is_empty() {
            return None;
        }
        let file = self.modules.resolve(importer, module)?;
        let qualified = if edge.metadata.contains_key("receiver") {
            format!("{file}::{symbol}.{}", edge.to)
        } else {
            format!("{file}::{symbol}")
        };
        self.qualified_symbol(&qualified, false)
    }

    /// Binds a Rust scoped `CALLS` (`config::load`, `Disk::open`) or an `IMPLEMENTS` trait
    /// path to the symbol its module path names, through the Cargo layouts and the file's
    /// `use` aliases (code-review-graph `_resolve_rust_scoped_call` and
    /// `_resolve_rust_type_target`). Only that exact qualified name binds; anything else
    /// (`Vec::new`, `fmt::Display`) is `None` and keeps its text.
    pub(crate) fn rust_path_target(&self, edge: &EdgeRelationship) -> Option<NodeId> {
        self.qualified_symbol(&self.rust_path_claim(edge)?, false)
    }

    /// The qualified name a Rust scoped call or trait path claims (`<file>::load`,
    /// `<file>::Disk.open`, `<file>::Tr`) once its module path names an indexed file, whether
    /// or not that symbol exists. `None` when the path names no file.
    pub(crate) fn rust_path_claim(&self, edge: &EdgeRelationship) -> Option<String> {
        let importer = &self.symbols.get(&edge.from)?.file;
        if !importer.ends_with(".rs") || self.is_path_qualified(&edge.to) {
            return None;
        }
        let aliases = self.rust_aliases.get(importer);
        let segments: Vec<&str> = edge.to.split("::").filter(|s| !s.is_empty()).collect();
        // The first segment through the file's `use` aliases (`cfg` -> `crate::config`).
        let expand = |segments: &[&str]| -> Option<Vec<String>> {
            let (first, rest) = segments.split_first()?;
            let import = aliases?.get(*first)?;
            Some(
                import
                    .split("::")
                    .chain(rest.iter().copied())
                    .map(str::to_string)
                    .collect(),
            )
        };
        match edge.edge_type {
            EdgeType::Calls => {
                let (method, prefix) = segments.split_last()?;
                if prefix.is_empty() || prefix == ["Self"] {
                    return None;
                }
                let prefix: Vec<String> = expand(prefix)
                    .unwrap_or_else(|| prefix.iter().map(|s| s.to_string()).collect());
                let file = self.modules.resolve(importer, &prefix.join("::"))?;
                let parent = (prefix.len() > 1)
                    .then(|| {
                        self.modules
                            .resolve(importer, &prefix[..prefix.len() - 1].join("::"))
                    })
                    .flatten();
                let last = prefix.last()?.as_str();
                let qualified =
                    if parent == Some(file) && !matches!(last, "crate" | "self" | "super") {
                        format!("{file}::{last}.{method}")
                    } else {
                        format!("{file}::{method}")
                    };
                Some(qualified)
            }
            EdgeType::Implements => {
                if let Some(imported) = expand(&segments)
                    && let Some(file) = self.modules.resolve(importer, &imported.join("::"))
                {
                    let name = imported.last()?;
                    return Some(format!("{file}::{name}"));
                }
                if segments.len() < 2 {
                    return None;
                }
                let file = self.modules.resolve(importer, &segments.join("::"))?;
                Some(format!("{file}::{}", segments.last()?))
            }
            _ => None,
        }
    }

    /// The symbol whose qualified name is exactly `qualified` (a top-level one if asked).
    fn qualified_symbol(&self, qualified: &str, top_level: bool) -> Option<NodeId> {
        self.aliases.get(qualified)?.iter().copied().find(|id| {
            self.symbols
                .get(id)
                .is_some_and(|s| s.qualified == qualified && !(top_level && s.has_parent))
        })
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
    use crate::rust_crates::RustCrates;
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
    fn an_exact_qualified_name_beats_a_shared_file_alias() {
        let mut class = node("a.java", "User");
        let mut constructor = node("a.java", "User");
        let caller = node("a.java", "create");
        class
            .metadata
            .attributes
            .insert("qualified_name".into(), "a.java::User".into());
        constructor
            .metadata
            .attributes
            .insert("qualified_name".into(), "a.java::User.User".into());
        constructor.set_deterministic_id("constructor");
        let catalog = SymbolCatalog::new(&[class.clone(), constructor, caller.clone()]);
        assert_eq!(
            catalog.exact(&edge(caller.id, "a.java::User"), &[]),
            Some(class.id)
        );
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

    fn file(path: &str) -> CodeNode {
        let mut file = node(path, path);
        file.node_type = Some(NodeType::Other("File".into()));
        file.set_deterministic_id(path);
        file
    }

    #[test]
    fn python_submodule_candidates_stay_only_when_they_name_a_file() {
        let a = file("a.py");
        let catalog = SymbolCatalog::new(&[
            a.clone(),
            file("pkg/__init__.py"),
            file("pkg/b.py"),
            file("ns/c.py"),
        ]);
        let import = |to: &str, statement: u32, submodule: bool| {
            let mut import = edge(a.id, to);
            import.edge_type = EdgeType::Imports;
            import.span = Some(codegraph_core::Span {
                start_byte: statement,
                end_byte: statement + 1,
            });
            if submodule {
                import.metadata.insert(
                    codegraph_parser::languages::crg_shape::PY_SUBMODULE_ATTR.into(),
                    "true".into(),
                );
            }
            import
        };
        let mut edges = vec![
            import("pkg", 1, false),
            import("pkg.b", 1, true),
            import("pkg.helper", 1, true),
            // Namespace package: `ns` has no file, its submodule does.
            import("ns", 2, false),
            import("ns.c", 2, true),
            import("json", 3, false),
            import("other", 4, false),
            import("other.q", 4, true),
            edge(a.id, "pkg.missing"),
        ];
        catalog.retain_python_imports(&mut edges);
        let kept: Vec<_> = edges.iter().map(|e| e.to.as_str()).collect();
        assert_eq!(
            kept,
            ["pkg", "pkg.b", "ns.c", "json", "other", "pkg.missing"]
        );
    }

    fn py(path: &str, qualified: &str, parent: Option<&str>) -> CodeNode {
        let name = qualified.rsplit(['.', ':']).next().unwrap();
        let mut node = node(path, name);
        node.metadata
            .attributes
            .insert("qualified_name".into(), format!("{path}::{qualified}"));
        if let Some(parent) = parent {
            node.metadata
                .attributes
                .insert("parent_name".into(), parent.into());
        }
        node.set_deterministic_id(qualified);
        node
    }

    fn py_edge(from: NodeId, to: &str, import: &str, receiver: bool) -> EdgeRelationship {
        let mut edge = edge(from, to);
        edge.metadata.insert(
            codegraph_parser::languages::crg_shape::PY_IMPORT_ATTR.into(),
            import.into(),
        );
        if receiver {
            edge.metadata.insert("receiver".into(), "x".into());
        }
        edge
    }

    #[test]
    fn python_import_bound_calls_reach_the_imported_definition() {
        let caller = py("a.py", "f", None);
        let helper = py("pkg/b.py", "helper", None);
        let thing = py("pkg/b.py", "Thing", None);
        let go = py("pkg/b.py", "Thing.go", Some("Thing"));
        let method_helper = py("pkg/c.py", "C.helper", Some("C"));
        let catalog = SymbolCatalog::new(&[
            file("a.py"),
            file("pkg/__init__.py"),
            file("pkg/b.py"),
            file("pkg/c.py"),
            caller.clone(),
            helper.clone(),
            thing.clone(),
            go.clone(),
            method_helper,
        ]);
        let target = |to, import, receiver| {
            catalog.py_import_target(&py_edge(caller.id, to, import, receiver))
        };
        // `from pkg import b; b.helper()`: `pkg.b` is a module, `helper` its top-level def.
        assert_eq!(target("helper", "pkg|b", true), Some(helper.id));
        // `from pkg.b import Thing; Thing.go()`: `pkg.b.Thing` is no module; `pkg.b` is.
        assert_eq!(target("go", "pkg.b|Thing", true), Some(go.id));
        // `import pkg.b as bm; bm.helper()`.
        assert_eq!(target("helper", "pkg.b|", true), Some(helper.id));
        // `from pkg.b import helper as hh; hh()`: no receiver, the symbol is `helper`.
        assert_eq!(target("hh", "pkg.b|helper", false), Some(helper.id));
        assert_eq!(target("Thing", "pkg.b|Thing", false), Some(thing.id));
        // A same-named method is not a top-level function.
        assert_eq!(target("helper", "pkg|c", true), None);
        // Unresolvable modules bind nothing.
        assert_eq!(target("loads", "json|", true), None);
        assert_eq!(target("go", "pkg.b|Other", true), None);
    }

    /// CRG `_PY_MODULE_BINDING_KEY`: `from pkg import b; b()` / `run(b)` names the module
    /// itself, so the bare name binds to the file node of `pkg/b.py`.
    #[test]
    fn python_bare_module_name_binds_to_its_file() {
        let caller = py("a.py", "f", None);
        let b_file = file("pkg/b.py");
        let catalog = SymbolCatalog::new(&[
            file("a.py"),
            file("pkg/__init__.py"),
            b_file.clone(),
            caller.clone(),
        ]);
        let target = |to, import, receiver| {
            catalog.py_import_target(&py_edge(caller.id, to, import, receiver))
        };
        assert_eq!(target("b", "pkg|b", false), Some(b_file.id));
        assert_eq!(target("bee", "pkg|b", false), Some(b_file.id));
        // A receiver call still names a symbol inside the module.
        assert_eq!(target("b", "pkg|b", true), None);
    }

    /// Two Cargo crates: `app` (binary) depends on `util-lib` by path.
    struct RustFixture {
        catalog: SymbolCatalog,
        run: CodeNode,
        local: CodeNode,
        load: CodeNode,
        tr: CodeNode,
        open: CodeNode,
        shout: CodeNode,
    }

    /// `uses` are the `(alias, path)` pairs of `use` declarations in `app/src/main.rs`.
    fn rust_fixture(uses: &[(&str, &str)]) -> RustFixture {
        let main = file("app/src/main.rs");
        let run = py("app/src/main.rs", "run", None);
        let local = py("app/src/main.rs", "Local", None);
        let load = py("app/src/config.rs", "load", None);
        let tr = py("app/src/config.rs", "Tr", None);
        let disk = py("app/src/store/disk.rs", "Disk", None);
        let open = py("app/src/store/disk.rs", "Disk.open", Some("Disk"));
        let shout = py("util-lib/src/text.rs", "shout", None);
        let paths = [
            "app/src/main.rs",
            "app/src/config.rs",
            "app/src/store.rs",
            "app/src/store/disk.rs",
            "util-lib/src/lib.rs",
            "util-lib/src/text.rs",
        ];
        let mut nodes: Vec<CodeNode> = paths[1..].iter().map(|path| file(path)).collect();
        nodes.extend([
            main.clone(),
            run.clone(),
            local.clone(),
            load.clone(),
            tr.clone(),
            disk,
            open.clone(),
            shout.clone(),
        ]);
        let mut catalog = SymbolCatalog::new(&nodes);
        let manifests = BTreeMap::from([
            (
                "app".to_string(),
                "[package]\nname = \"app\"\n[dependencies]\n\
                 util-lib = { path = \"../util-lib\" }\n"
                    .to_string(),
            ),
            (
                "util-lib".to_string(),
                "[package]\nname = \"util-lib\"\n".to_string(),
            ),
        ]);
        let files: BTreeSet<String> = paths.iter().map(|p| p.to_string()).collect();
        catalog.set_rust_crates(RustCrates::from_manifests(&manifests, &files));
        let imports: Vec<_> = uses
            .iter()
            .map(|(alias, path)| {
                let mut import = edge(main.id, path);
                import.edge_type = EdgeType::Imports;
                import.metadata.insert(
                    codegraph_parser::languages::crg_shape::RUST_ALIAS_ATTR.into(),
                    alias.to_string(),
                );
                import
            })
            .collect();
        catalog.link_imports(&imports);
        RustFixture {
            catalog,
            run,
            local,
            load,
            tr,
            open,
            shout,
        }
    }

    fn implements(from: NodeId, to: &str) -> EdgeRelationship {
        let mut edge = edge(from, to);
        edge.edge_type = EdgeType::Implements;
        edge
    }

    #[test]
    fn rust_scoped_call_binds_to_the_function_in_the_module_file() {
        let f = rust_fixture(&[("config", "crate::config")]);
        assert_eq!(
            f.catalog.rust_path_target(&edge(f.run.id, "config::load")),
            Some(f.load.id)
        );
        // Written in full, without the alias.
        assert_eq!(
            f.catalog
                .rust_path_target(&edge(f.run.id, "crate::config::load")),
            Some(f.load.id)
        );
        // The module resolves but holds no such function: no edge.
        assert_eq!(
            f.catalog
                .rust_path_target(&edge(f.run.id, "config::missing")),
            None
        );
    }

    /// code-review-graph qualifies the call into the module file anyway, then
    /// `_settle_unbound_endpoints` turns the missing endpoint back into the plain name.
    #[test]
    fn rust_scoped_call_to_a_missing_item_keeps_the_plain_name() {
        let f = rust_fixture(&[("config", "crate::config")]);
        let missing = edge(f.run.id, "config::missing");
        assert_eq!(
            f.catalog.rust_path_claim(&missing).as_deref(),
            Some("app/src/config.rs::missing")
        );
        assert_eq!(
            crate::indexer::ProjectIndexer::unbound_target(&f.catalog, &missing),
            "missing"
        );
        // A prefix that names no module file keeps the text.
        let vec = edge(f.run.id, "Vec::new");
        assert_eq!(f.catalog.rust_path_claim(&vec), None);
        assert_eq!(
            crate::indexer::ProjectIndexer::unbound_target(&f.catalog, &vec),
            "Vec::new"
        );
    }

    #[test]
    fn rust_associated_call_binds_to_the_type_in_the_module_file() {
        let f = rust_fixture(&[("Disk", "crate::store::disk::Disk")]);
        assert_eq!(
            f.catalog.rust_path_target(&edge(f.run.id, "Disk::open")),
            Some(f.open.id)
        );
        assert_eq!(
            f.catalog
                .rust_path_target(&edge(f.run.id, "crate::store::disk::Disk::open")),
            Some(f.open.id)
        );
    }

    #[test]
    fn rust_alias_import_is_expanded() {
        let f = rust_fixture(&[("cfg", "crate::config")]);
        assert_eq!(
            f.catalog.rust_path_target(&edge(f.run.id, "cfg::load")),
            Some(f.load.id)
        );
        // A glob import names no alias.
        let g = rust_fixture(&[("*", "crate::config")]);
        assert_eq!(g.catalog.rust_path_target(&edge(g.run.id, "*::load")), None);
    }

    #[test]
    fn rust_trait_impl_binds_to_the_trait_in_another_file() {
        let f = rust_fixture(&[("Tr", "crate::config::Tr"), ("Trait", "crate::config::Tr")]);
        assert_eq!(
            f.catalog.rust_path_target(&implements(f.local.id, "Tr")),
            Some(f.tr.id)
        );
        // `use crate::config::Tr as Trait`: the trait keeps its own name.
        assert_eq!(
            f.catalog.rust_path_target(&implements(f.local.id, "Trait")),
            Some(f.tr.id)
        );
        assert_eq!(
            f.catalog
                .rust_path_target(&implements(f.local.id, "crate::config::Tr")),
            Some(f.tr.id)
        );
        // Not imported, not a path: stays text.
        assert_eq!(
            f.catalog.rust_path_target(&implements(f.local.id, "Other")),
            None
        );
    }

    #[test]
    fn rust_std_and_unknown_paths_stay_text() {
        let f = rust_fixture(&[("fmt", "std::fmt"), ("Serialize", "serde::Serialize")]);
        assert_eq!(
            f.catalog.rust_path_target(&edge(f.run.id, "Vec::new")),
            None
        );
        assert_eq!(
            f.catalog.rust_path_target(&edge(f.run.id, "fmt::format")),
            None
        );
        for to in ["fmt::Display", "serde::Serialize", "Serialize"] {
            assert_eq!(
                f.catalog.rust_path_target(&implements(f.local.id, to)),
                None,
                "{to}"
            );
        }
        // Only CALLS and IMPLEMENTS of `.rs` importers apply.
        let mut uses = edge(f.run.id, "config::load");
        uses.edge_type = EdgeType::Uses;
        assert_eq!(f.catalog.rust_path_target(&uses), None);
    }

    #[test]
    fn rust_dependency_crate_call_binds_into_the_other_crate() {
        let f = rust_fixture(&[]);
        assert_eq!(
            f.catalog
                .rust_path_target(&edge(f.run.id, "util_lib::text::shout")),
            Some(f.shout.id)
        );
        assert_eq!(
            f.catalog
                .rust_path_target(&edge(f.run.id, "util-lib::text::shout")),
            Some(f.shout.id)
        );
        // `text` is a module of `util-lib`, not of `app`.
        assert_eq!(
            f.catalog.rust_path_target(&edge(f.run.id, "text::shout")),
            None
        );
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
