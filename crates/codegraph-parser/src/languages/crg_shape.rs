// ABOUTME: Reshapes an extraction into code-review-graph's graph shape for every language.
// ABOUTME: Adds the File node, class-qualified names and CONTAINS edges from file/class to members.

use super::test_detection;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

/// Set on every node whose `qualified_name` this pass rewrote.
pub const SHAPED_ATTR: &str = "crg_shaped";
/// The `qualified_name` a rewritten node had before this pass, when it had one.
pub const PRE_SHAPE_QN_ATTR: &str = "pre_shape_qualified_name";

/// Set by an extractor on an edge made outside every function; this pass moves its source to
/// the `File` node (the extractor records `NodeId::nil()`) and removes the key.
pub const FILE_SCOPE_ATTR: &str = "crg_file_scope";

/// Set by an extractor on a `Function` node it made from a test runner call (`it('x', ...)`).
/// This pass turns it into a `Test` and links it from the function or runner around it.
pub const TEST_RUNNER_ATTR: &str = "crg_test_runner";

/// Comma-separated annotation/decorator names an extractor recorded on a node, without the
/// `@` (Java `@Test`); test detection reads them like CRG's `decorators`.
pub const DECORATORS_ATTR: &str = "decorators";

/// Set by an extractor on an edge whose target text code-review-graph keeps; the indexer
/// only binds it to a symbol with that exact alias.
pub const VERBATIM_TARGET_ATTR: &str = "crg_verbatim_target";

/// Set by an extractor on a method call whose receiver's type it cannot know (Rust `v.map()`):
/// like code-review-graph, the indexer never binds it to a method, only to a free function.
pub const UNTYPED_RECEIVER_ATTR: &str = "crg_untyped_receiver";

/// Set by an extractor on a call whose receiver it cannot bind (Python `x.m()`, `self.repo.m()`):
/// like code-review-graph, the indexer never binds it, not even to a free function.
pub const UNBOUND_ATTR: &str = "crg_unbound";

/// Local name a Rust `use` binds (`use x::Y as Z` -> `Z`), on `Imports` edges.
pub const RUST_ALIAS_ATTR: &str = "crg_rust_alias";

/// Set by an extractor on a Python call bound through an import, as `"<module>|<symbol>"`
/// (`from M import X` -> `"M|X"`, `import a.b as c` -> `"a.b|"`); the indexer resolves the
/// module with its `ModuleIndex`.
pub const PY_IMPORT_ATTR: &str = "crg_py_import";

/// Set by the Python extractor on the `M.x` candidate of `from M import x`: the indexer keeps
/// it only when it resolves to a file (`x` is a submodule, not a symbol), like
/// code-review-graph's `_python_from_import_targets`.
pub const PY_SUBMODULE_ATTR: &str = "crg_py_submodule";

/// The node kind (`{:?}` of its `NodeType`) a node had before this pass turned it into a test.
pub const PRE_SHAPE_KIND_ATTR: &str = "pre_shape_kind";

/// The `qualified_name` the indexer must use when deriving node ids: the value from before
/// this pass, so node ids stay byte-identical to the unshaped extraction.
pub fn id_qualified_name(node: &CodeNode) -> Option<&String> {
    let attributes = &node.metadata.attributes;
    if attributes.contains_key(SHAPED_ATTR) {
        attributes.get(PRE_SHAPE_QN_ATTR)
    } else {
        attributes.get("qualified_name")
    }
}

/// The node kind the indexer must hash into node ids: the kind from before this pass, so a
/// function this pass marks as a `Test` keeps its id.
pub fn id_node_type(node: &CodeNode) -> String {
    if let Some(kind) = node.metadata.attributes.get(PRE_SHAPE_KIND_ATTR) {
        return kind.clone();
    }
    node.node_type
        .as_ref()
        .map(|kind| format!("{kind:?}"))
        .unwrap_or_else(|| "Unknown".into())
}

/// Lexically normalizes a file path to the root-relative key used in `qualified_name`s.
/// `.`, `..`, repeated separators and an absolute spelling under the current directory all
/// collapse to the same string; an absolute path outside it keeps its components.
pub fn default_rel_path(file_path: &str) -> String {
    let raw = Path::new(file_path);
    let cwd = std::env::current_dir().ok();
    let relative = if raw.is_absolute() {
        match cwd.as_deref().and_then(|root| raw.strip_prefix(root).ok()) {
            Some(under_cwd) => under_cwd.to_path_buf(),
            None => return file_path.trim_end_matches('/').to_string(),
        }
    } else {
        raw.to_path_buf()
    };
    let mut parts: Vec<String> = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
        }
    }
    let joined = parts.join("/");
    if joined.is_empty() {
        file_path.to_string()
    } else {
        joined
    }
}

fn is_container(kind: &NodeType) -> bool {
    matches!(
        kind,
        NodeType::Class | NodeType::Struct | NodeType::Trait | NodeType::Interface | NodeType::Enum
    )
}

fn is_member(kind: &NodeType) -> bool {
    is_container(kind) || matches!(kind, NodeType::Function | NodeType::Type)
}

fn is_test_runner(node: &CodeNode) -> bool {
    node.metadata.attributes.contains_key(TEST_RUNNER_ATTR)
}

/// `(start, end)` positions as `(line, column)` pairs.
fn extent(node: &CodeNode) -> ((u32, u32), (u32, u32)) {
    let loc = &node.location;
    (
        (loc.line, loc.column),
        (
            loc.end_line.unwrap_or(loc.line),
            loc.end_column.unwrap_or(loc.column),
        ),
    )
}

enum Parent {
    /// A class-like node of this extraction.
    Node(usize),
    /// A type that owns a method but is not defined in this file.
    Name(String),
}

/// The type a method is declared on outside the type's body: Rust `method_of` (the `impl`
/// target) or Go `receiver` (`(s *Stack[K])`), reduced to its last bare name (`Stack`).
fn owner_type(node: &CodeNode) -> Option<String> {
    let attributes = &node.metadata.attributes;
    let raw = attributes
        .get("method_of")
        .or_else(|| attributes.get("receiver"))?;
    let raw = raw.trim().trim_start_matches('(').trim_end_matches(')');
    let raw = raw.split(['<', '[']).next().unwrap_or(raw);
    let name = raw
        .split_whitespace()
        .last()?
        .trim_start_matches(['*', '&', '('])
        .rsplit("::")
        .next()?
        .trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// CRG's File `line_end`: `source.count("\n") + 1`.
pub fn line_count(content: &str) -> u32 {
    content.matches('\n').count() as u32 + 1
}

/// Adds the `File` node, rewrites `qualified_name`/`parent_name` of functions and types to
/// `<file>::<Outer.Inner>.<name>` and links each to its file or innermost enclosing class with
/// `CONTAINS`, as code-review-graph does for every language. Import nodes are dropped and
/// their `Imports` edges leave the file instead; functions that code-review-graph would call
/// tests (judged on `rel_path`, relative to the project root) become `Test` nodes.
pub fn shape_for_crg(
    result: &mut ExtractionResult,
    file_path: &str,
    rel_path: &str,
    language: Language,
    line_count: u32,
) {
    // The graph key is root-relative and normalized; the original `file_path` stays only in
    // locations, where it must still point at the file on disk.
    let rel_path = default_rel_path(rel_path);
    let imports: HashSet<NodeId> = result
        .nodes
        .iter()
        .filter(|n| n.node_type == Some(NodeType::Import))
        .map(|n| n.id)
        .collect();
    result
        .nodes
        .retain(|n| n.node_type != Some(NodeType::Import));

    let members: Vec<usize> = (0..result.nodes.len())
        .filter(|&i| result.nodes[i].node_type.as_ref().is_some_and(is_member))
        .collect();
    let containers: Vec<usize> = members
        .iter()
        .copied()
        .filter(|&i| result.nodes[i].node_type.as_ref().is_some_and(is_container))
        .collect();

    // Innermost enclosing container of each member, by span.
    let mut parent_of: HashMap<usize, Parent> = members
        .iter()
        .filter_map(|&child| {
            let (start, end) = extent(&result.nodes[child]);
            containers
                .iter()
                .copied()
                .filter(|&c| c != child)
                .filter(|&c| {
                    let (c_start, c_end) = extent(&result.nodes[c]);
                    c_start <= start && end <= c_end && (c_start, c_end) != (start, end)
                })
                .max_by_key(|&c| {
                    let (c_start, c_end) = extent(&result.nodes[c]);
                    (c_start, std::cmp::Reverse(c_end))
                })
                .map(|c| (child, Parent::Node(c)))
        })
        .collect();

    // Methods declared outside their type (Rust `impl`, Go receivers) belong to that type,
    // which CRG names `Owner.method` whether or not the type is defined in this file.
    for &i in &members {
        if parent_of.contains_key(&i) || result.nodes[i].node_type != Some(NodeType::Function) {
            continue;
        }
        let Some(owner) = owner_type(&result.nodes[i]) else {
            continue;
        };
        let defined = containers
            .iter()
            .copied()
            .find(|&c| result.nodes[c].name.as_str() == owner && !parent_of.contains_key(&c));
        let parent = defined.map_or(Parent::Name(owner), Parent::Node);
        parent_of.insert(i, parent);
    }

    let python = language == Language::Python;
    // Dotted class path (`Outer.Inner`) of each member, outermost first.
    let mut paths: HashMap<usize, String> = HashMap::new();
    fn path_of(
        i: usize,
        nodes: &[CodeNode],
        parent_of: &HashMap<usize, Parent>,
        paths: &mut HashMap<usize, String>,
        depth: usize,
    ) -> String {
        if let Some(path) = paths.get(&i) {
            return path.clone();
        }
        let name = nodes[i].name.to_string();
        let path = match parent_of.get(&i) {
            Some(Parent::Node(p)) if depth < 64 => {
                format!("{}.{name}", path_of(*p, nodes, parent_of, paths, depth + 1))
            }
            Some(Parent::Name(owner)) => format!("{owner}.{name}"),
            _ => name,
        };
        paths.insert(i, path.clone());
        path
    }
    for &i in &members {
        path_of(i, &result.nodes, &parent_of, &mut paths, 0);
    }

    let mut file_node = CodeNode::new(
        file_path.to_string(),
        Some(NodeType::Other("File".into())),
        Some(language),
        Location {
            file_path: file_path.to_string(),
            line: 1,
            column: 0,
            end_line: Some(line_count),
            end_column: Some(0),
        },
    );
    file_node
        .metadata
        .attributes
        .insert("qualified_name".into(), rel_path.clone());
    if test_detection::is_test_file(&rel_path) {
        file_node
            .metadata
            .attributes
            .insert("is_test".into(), "true".into());
    }

    // A test runner node hangs off the innermost function or runner around it (CRG uses the
    // enclosing function), not off its class or file.
    let runner_parents: HashMap<usize, NodeId> = members
        .iter()
        .copied()
        .filter(|&i| is_test_runner(&result.nodes[i]))
        .filter_map(|child| {
            let (start, end) = extent(&result.nodes[child]);
            members
                .iter()
                .copied()
                .filter(|&f| f != child && result.nodes[f].node_type == Some(NodeType::Function))
                .filter(|&f| {
                    let (f_start, f_end) = extent(&result.nodes[f]);
                    f_start <= start && end <= f_end && (f_start, f_end) != (start, end)
                })
                .max_by_key(|&f| {
                    let (f_start, f_end) = extent(&result.nodes[f]);
                    (f_start, std::cmp::Reverse(f_end))
                })
                .map(|f| (child, result.nodes[f].id))
        })
        .collect();

    // Python (CRG): inside a nested class the scope is only its leaf name, unless two classes
    // of the file share that leaf under different chains; a class nested in a class hangs off
    // the file, not off its parent.
    let scopes: HashMap<usize, String> = containers
        .iter()
        .map(|&p| {
            let scope = if !python || !parent_of.contains_key(&p) {
                paths[&p].clone()
            } else {
                let leaf = result.nodes[p].name.as_str();
                let ambiguous = containers.iter().any(|&c| {
                    c != p && result.nodes[c].name.as_str() == leaf && paths[&c] != paths[&p]
                });
                if ambiguous {
                    paths[&p].clone()
                } else {
                    leaf.to_string()
                }
            };
            (p, scope)
        })
        .collect();

    for &i in &members {
        let mut qualified = format!("{rel_path}::{}", paths[&i]);
        let (parent, mut from) = match parent_of.get(&i) {
            Some(Parent::Node(p)) if python => {
                let scope = scopes[p].clone();
                qualified = format!("{rel_path}::{scope}.{}", result.nodes[i].name);
                let from = if result.nodes[i].node_type == Some(NodeType::Class) {
                    file_node.id
                } else {
                    result.nodes[*p].id
                };
                (Some(scope), from)
            }
            Some(Parent::Node(p)) => (Some(paths[p].clone()), result.nodes[*p].id),
            Some(Parent::Name(owner)) => (Some(owner.clone()), file_node.id),
            None => (None, file_node.id),
        };
        if let Some(&runner_parent) = runner_parents.get(&i) {
            from = runner_parent;
        }
        let node = &mut result.nodes[i];
        let attributes = &mut node.metadata.attributes;
        if let Some(original) = attributes.insert("qualified_name".into(), qualified.clone()) {
            attributes.insert(PRE_SHAPE_QN_ATTR.into(), original);
        }
        attributes.insert(SHAPED_ATTR.into(), "true".into());
        if let Some(parent) = parent {
            attributes.insert("parent_name".into(), parent);
        }
        result.edges.push(EdgeRelationship {
            from,
            to: qualified,
            edge_type: EdgeType::Contains,
            metadata: HashMap::from([
                ("source_file".into(), rel_path.clone()),
                ("target_node_id".into(), node.id.to_string()),
                ("line".into(), node.location.line.to_string()),
                ("analyzer".into(), "crg_shape".into()),
            ]),
            span: None,
        });
    }

    // Only the extractors that record decorators (DECORATORS_ATTR) let them decide.
    for &i in &members {
        let node = &mut result.nodes[i];
        let decorators: Vec<&str> = node
            .metadata
            .attributes
            .get(DECORATORS_ATTR)
            .map(|d| d.split(',').collect())
            .unwrap_or_default();
        if node.node_type != Some(NodeType::Function)
            || !(is_test_runner(node)
                || test_detection::is_test_function(node.name.as_str(), &rel_path, &decorators))
        {
            continue;
        }
        let attributes = &mut node.metadata.attributes;
        attributes.insert(
            PRE_SHAPE_KIND_ATTR.into(),
            format!("{:?}", NodeType::Function),
        );
        attributes.insert("is_test".into(), "true".into());
        node.node_type = Some(NodeType::Other("Test".into()));
    }

    if test_detection::is_test_file(&rel_path) {
        for &i in &members {
            result.nodes[i]
                .metadata
                .attributes
                .insert("is_test".into(), "true".into());
        }
    }

    for edge in &mut result.edges {
        if imports.contains(&edge.from) || edge.metadata.remove(FILE_SCOPE_ATTR).is_some() {
            edge.from = file_node.id;
        }
    }

    result.nodes.insert(0, file_node);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::extract_for_language;
    use std::collections::BTreeSet;

    fn extract_python(source: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_for_language(&Language::Python, &tree, source, "m.py").unwrap()
    }

    fn qualified_names(result: &ExtractionResult) -> BTreeSet<String> {
        result
            .nodes
            .iter()
            .filter_map(|n| n.metadata.attributes.get("qualified_name").cloned())
            .collect()
    }

    fn contains_edges(result: &ExtractionResult) -> BTreeSet<(String, String)> {
        result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Contains)
            .map(|e| {
                let from = result.nodes.iter().find(|n| n.id == e.from).unwrap();
                (
                    from.metadata.attributes["qualified_name"].clone(),
                    e.to.clone(),
                )
            })
            .collect()
    }

    fn set(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        items
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn methods_get_class_prefix_and_contains() {
        let r = extract_python(
            "class A:\n    def run(self):\n        pass\n\n\
             class B:\n    def run(self):\n        pass\n\n\
             def top():\n    pass\n",
        );
        let qns = qualified_names(&r);
        assert!(qns.contains("m.py::A.run") && qns.contains("m.py::B.run"));
        assert_eq!(
            contains_edges(&r),
            set(&[
                ("m.py", "m.py::A"),
                ("m.py", "m.py::B"),
                ("m.py", "m.py::top"),
                ("m.py::A", "m.py::A.run"),
                ("m.py::B", "m.py::B.run"),
            ])
        );
        let run_a = r
            .nodes
            .iter()
            .find(|n| {
                n.metadata
                    .attributes
                    .get("qualified_name")
                    .map(String::as_str)
                    == Some("m.py::A.run")
            })
            .unwrap();
        assert_eq!(run_a.name.as_str(), "run");
        assert_eq!(run_a.metadata.attributes["parent_name"], "A");
    }

    #[test]
    fn nested_classes_are_dotted() {
        let r = extract_python(
            "class Outer:\n    class Inner:\n        def go(self):\n            pass\n",
        );
        let qns = qualified_names(&r);
        assert!(qns.contains("m.py::Outer.Inner"));
        assert!(qns.contains("m.py::Inner.go"));
        assert!(contains_edges(&r).contains(&(
            "m.py::Outer.Inner".to_string(),
            "m.py::Inner.go".to_string()
        )));
    }

    #[test]
    fn python_nested_classes_scope_members_by_leaf() {
        let r = extract_python(
            "class Child:\n    class Inner:\n        def go(self):\n            pass\n        class Deep:\n            pass\n",
        );
        let qns = qualified_names(&r);
        for q in [
            "m.py::Child",
            "m.py::Child.Inner",
            "m.py::Inner.go",
            "m.py::Inner.Deep",
        ] {
            assert!(qns.contains(q), "missing {q}: {qns:?}");
        }
        let edges = contains_edges(&r);
        for (a, b) in [
            ("m.py", "m.py::Child"),
            ("m.py", "m.py::Child.Inner"),
            ("m.py::Child.Inner", "m.py::Inner.go"),
            ("m.py", "m.py::Inner.Deep"),
        ] {
            assert!(
                edges.contains(&(a.to_string(), b.to_string())),
                "missing {a}->{b}: {edges:?}"
            );
        }
        let go = r.nodes.iter().find(|n| n.name.as_str() == "go").unwrap();
        assert_eq!(go.metadata.attributes["parent_name"], "Inner");
    }

    #[test]
    fn python_ambiguous_leaf_keeps_the_full_chain() {
        let r = extract_python(
            "class A:\n    class In:\n        def go(self): pass\nclass B:\n    class In:\n        def go(self): pass\n",
        );
        let qns = qualified_names(&r);
        assert!(qns.contains("m.py::A.In.go"), "{qns:?}");
        assert!(qns.contains("m.py::B.In.go"), "{qns:?}");
        assert!(
            contains_edges(&r).contains(&("m.py::A.In".to_string(), "m.py::A.In.go".to_string()))
        );
        let go = r
            .nodes
            .iter()
            .find(|n| {
                n.metadata
                    .attributes
                    .get("qualified_name")
                    .map(String::as_str)
                    == Some("m.py::A.In.go")
            })
            .unwrap();
        assert_eq!(go.metadata.attributes["parent_name"], "A.In");
    }

    #[test]
    fn every_node_of_a_test_file_is_flagged() {
        let source =
            "class TestA:\n    def setUp(self):\n        pass\n\ndef helper():\n    pass\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let r = crate::languages::extract_for_language_at(
            &Language::Python,
            &tree,
            source,
            "./tests/test_m.py",
            "tests/test_m.py",
            Default::default(),
        )
        .unwrap();
        for name in ["TestA", "setUp", "helper"] {
            let n = r.nodes.iter().find(|n| n.name.as_str() == name).unwrap();
            assert_eq!(n.metadata.attributes["is_test"], "true", "{name}");
        }
        let helper = r
            .nodes
            .iter()
            .find(|n| n.name.as_str() == "helper")
            .unwrap();
        assert_eq!(helper.node_type, Some(NodeType::Function));
        let class = r.nodes.iter().find(|n| n.name.as_str() == "TestA").unwrap();
        assert_eq!(class.node_type, Some(NodeType::Class));
    }

    #[test]
    fn nested_function_is_child_of_file() {
        let r = extract_python("def outer():\n    def inner():\n        pass\n    inner()\n");
        let qns = qualified_names(&r);
        assert!(qns.contains("m.py::outer") && qns.contains("m.py::inner"));
        assert_eq!(
            contains_edges(&r),
            set(&[("m.py", "m.py::outer"), ("m.py", "m.py::inner")])
        );
    }

    #[test]
    fn empty_file_still_has_file_node() {
        let r = extract_python("");
        let files: Vec<_> = r
            .nodes
            .iter()
            .filter(|n| n.node_type == Some(NodeType::Other("File".into())))
            .collect();
        assert_eq!(files.len(), 1);
        let file = files[0];
        assert_eq!(file.name.as_str(), "m.py");
        assert_eq!(file.metadata.attributes["qualified_name"], "m.py");
        assert_eq!((file.location.line, file.location.end_line), (1, Some(1)));
    }

    #[test]
    fn file_node_spans_every_line() {
        let r = extract_python("x = 1\ny = 2\nz = 3\n");
        let file = r
            .nodes
            .iter()
            .find(|n| n.node_type == Some(NodeType::Other("File".into())))
            .unwrap();
        assert_eq!((file.location.line, file.location.end_line), (1, Some(4)));
    }

    #[test]
    fn contains_edges_carry_the_child_id() {
        let r = extract_python("class A:\n    def run(self):\n        pass\n");
        for edge in r.edges.iter().filter(|e| e.edge_type == EdgeType::Contains) {
            let target = edge.metadata["target_node_id"].clone();
            let child = r.nodes.iter().find(|n| n.id.to_string() == target).unwrap();
            assert_eq!(child.metadata.attributes["qualified_name"], edge.to);
        }
    }

    fn file_node(result: &ExtractionResult) -> &CodeNode {
        result
            .nodes
            .iter()
            .find(|n| n.node_type == Some(NodeType::Other("File".into())))
            .unwrap()
    }

    #[test]
    fn import_becomes_file_edge() {
        let r = extract_python("import os\n\ndef f():\n    pass\n");
        assert!(
            !r.nodes
                .iter()
                .any(|n| n.node_type == Some(NodeType::Import)),
            "{:?}",
            r.nodes
        );
        let file = file_node(&r);
        assert!(!file.metadata.attributes.contains_key("is_test"));
        let imports: Vec<_> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Imports)
            .map(|e| (e.from, e.to.as_str()))
            .collect();
        assert_eq!(imports, vec![(file.id, "os")]);
    }

    #[test]
    fn test_function_is_test_kind() {
        let source = "def test_x():\n    helper()\n\ndef helper():\n    pass\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let r = crate::languages::extract_for_language_at(
            &Language::Python,
            &tree,
            source,
            "./tests/test_m.py",
            "tests/test_m.py",
            Default::default(),
        )
        .unwrap();
        let test_x = r
            .nodes
            .iter()
            .find(|n| n.name.as_str() == "test_x")
            .unwrap();
        assert_eq!(test_x.node_type, Some(NodeType::Other("Test".into())));
        assert_eq!(test_x.metadata.attributes["is_test"], "true");
        assert_eq!(
            test_x.metadata.attributes["qualified_name"],
            "tests/test_m.py::test_x"
        );
        assert_eq!(id_node_type(test_x), "Function");
        assert_eq!(file_node(&r).metadata.attributes["is_test"], "true");
        let helper = r
            .nodes
            .iter()
            .find(|n| n.name.as_str() == "helper")
            .unwrap();
        assert_eq!(helper.node_type, Some(NodeType::Function));
        // Every node of a test file is flagged, whatever its kind.
        assert_eq!(helper.metadata.attributes["is_test"], "true");
        assert_eq!(id_node_type(helper), "Function");
        // Calls made from a test keep their source node.
        assert!(
            r.edges
                .iter()
                .any(|e| e.edge_type == EdgeType::Calls && e.from == test_x.id)
        );
    }

    #[test]
    fn runner_names_are_tests_only_in_test_files() {
        let source = "function it() {}\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let kind = |rel: &str| {
            let r = crate::languages::extract_for_language_at(
                &Language::TypeScript,
                &tree,
                source,
                rel,
                rel,
                Default::default(),
            )
            .unwrap();
            r.nodes
                .iter()
                .find(|n| n.name.as_str() == "it")
                .and_then(|n| n.node_type.clone())
        };
        assert_eq!(kind("src/a.ts"), Some(NodeType::Function));
        assert_eq!(kind("src/a.test.ts"), Some(NodeType::Other("Test".into())));
    }

    fn extract(
        language: Language,
        ts: tree_sitter::Language,
        source: &str,
        path: &str,
    ) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts).unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_for_language(&language, &tree, source, path).unwrap()
    }

    #[test]
    fn rust_impl_methods_are_owned_by_their_type() {
        let source = "struct A;\nstruct B;\nimpl A {\n    fn new() -> A { A }\n}\n\
                      impl<T> Clone for B<T> {\n    fn new() -> B { B }\n}\n";
        let r = extract(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            source,
            "m.rs",
        );
        let qns = qualified_names(&r);
        assert!(
            qns.contains("m.rs::A.new") && qns.contains("m.rs::B.new"),
            "{qns:?}"
        );
        assert!(!qns.iter().any(|q| q.contains("@L")));
        let edges = contains_edges(&r);
        assert!(
            edges.contains(&("m.rs::A".into(), "m.rs::A.new".into())),
            "{edges:?}"
        );
        assert!(
            edges.contains(&("m.rs::B".into(), "m.rs::B.new".into())),
            "{edges:?}"
        );
        let new_a = r
            .nodes
            .iter()
            .find(|n| {
                n.metadata
                    .attributes
                    .get("qualified_name")
                    .map(String::as_str)
                    == Some("m.rs::A.new")
            })
            .unwrap();
        assert_eq!(new_a.metadata.attributes["parent_name"], "A");
    }

    #[test]
    fn rust_impl_for_a_type_outside_the_file_hangs_off_the_file() {
        let source = "impl Foreign {\n    fn go(&self) {}\n}\n";
        let r = extract(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            source,
            "m.rs",
        );
        assert!(qualified_names(&r).contains("m.rs::Foreign.go"));
        assert!(contains_edges(&r).contains(&("m.rs".into(), "m.rs::Foreign.go".into())));
    }

    #[test]
    fn go_receiver_methods_are_owned_by_their_type() {
        let source = "package p\n\ntype T struct{}\ntype U struct{}\n\n\
                      func (t *T) Save() bool { return true }\n\
                      func (U) Save() bool { return true }\n\
                      func (s *Stack[K]) Push() {}\n";
        let r = extract(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            source,
            "m.go",
        );
        let qns = qualified_names(&r);
        assert!(
            qns.contains("m.go::T.Save") && qns.contains("m.go::U.Save"),
            "{qns:?}"
        );
        assert!(qns.contains("m.go::Stack.Push"), "{qns:?}");
        assert!(!qns.iter().any(|q| q.contains("@L")));
        let edges = contains_edges(&r);
        assert!(
            edges.contains(&("m.go::T".into(), "m.go::T.Save".into())),
            "{edges:?}"
        );
        assert!(
            edges.contains(&("m.go".into(), "m.go::Stack.Push".into())),
            "{edges:?}"
        );
    }

    #[test]
    fn qualified_names_ignore_path_spelling() {
        let source = "def go():\n    pass\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let absolute = std::env::current_dir().unwrap().join("fx/m.py");
        let spellings = [
            "fx/m.py".to_string(),
            "./fx//m.py".to_string(),
            "fx/../fx/m.py".to_string(),
            absolute.to_string_lossy().into_owned(),
        ];
        let sets: Vec<BTreeSet<String>> = spellings
            .iter()
            .map(|path| {
                let result = extract_for_language(&Language::Python, &tree, source, path).unwrap();
                qualified_names(&result)
            })
            .collect();
        assert!(
            sets.iter().all(|set| set == &sets[0]),
            "path spellings produced different qualified names: {sets:?}"
        );
        assert!(
            sets[0]
                .iter()
                .all(|qn| !qn.contains("..") && !qn.contains("//")),
            "non-canonical qualified name: {:?}",
            sets[0]
        );
    }

    #[test]
    fn id_qualified_name_is_the_pre_shape_value() {
        let r = extract_python("class A:\n    def run(self):\n        pass\n");
        let run = r.nodes.iter().find(|n| n.name.as_str() == "run").unwrap();
        assert_eq!(run.metadata.attributes["qualified_name"], "m.py::A.run");
        assert_eq!(id_qualified_name(run), None);

        let source = "struct S;\nimpl S {\n    fn go(&self) {}\n}\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let r = extract_for_language(&Language::Rust, &tree, source, "m.rs").unwrap();
        let go = r.nodes.iter().find(|n| n.name.as_str() == "go").unwrap();
        assert_eq!(go.metadata.attributes["qualified_name"], "m.rs::S.go");
        assert_eq!(
            id_qualified_name(go).map(String::as_str),
            Some("m.rs::impl<S>::go")
        );
    }
}
