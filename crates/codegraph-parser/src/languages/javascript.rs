// ABOUTME: TypeScript/JavaScript extractor producing code-review-graph's declarations and calls.
// ABOUTME: Tracks the enclosing named function so calls in anonymous callbacks stay with it.

use super::{crg_shape, test_detection};
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::HashMap;
use tree_sitter::{Node, Tree};

/// TypeScript/JavaScript extractor with unified node+edge extraction.
pub struct TypeScriptExtractor;

impl TypeScriptExtractor {
    /// Extracts nodes and edges in one AST traversal. `test_file` says whether code-review-graph
    /// would call the file a test file (judged on the project-relative path).
    pub fn extract_with_edges(
        tree: &Tree,
        content: &str,
        file_path: &str,
        language: Language,
        test_file: bool,
    ) -> ExtractionResult {
        let mut collector = TypeScriptCollector::new(content, file_path, language, test_file);
        collector.walk(tree.root_node());
        collector.into_result()
    }
}

impl super::LanguageExtractor for TypeScriptExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        TypeScriptExtractor::extract_with_edges(
            tree,
            content,
            file_path,
            Language::TypeScript,
            false,
        )
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[
            EdgeType::Imports,
            EdgeType::Calls,
            EdgeType::Extends,
            EdgeType::Implements,
        ]
    }

    fn language() -> Language {
        Language::TypeScript
    }
}

/// CRG `_CLASS_TYPES["typescript"]`; all become `Class` nodes in code-review-graph.
const CLASS_KINDS: &[&str] = &[
    "class_declaration",
    "abstract_class_declaration",
    "class",
    "interface_declaration",
    "type_alias_declaration",
    "enum_declaration",
];

/// CRG `_FUNCTION_TYPES["typescript"]` that carry their own name.
const FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "method_definition",
    "abstract_method_signature",
];

/// CRG `_TEST_MODIFIER_SUFFIXES`: `describe.only`, `it.skip`, ... run as their base runner.
const TEST_MODIFIER_SUFFIXES: &[&str] = &["only", "skip", "each", "todo", "concurrent", "failing"];

/// CRG `_MAX_TEST_DESCRIPTION_LEN`.
const MAX_TEST_DESCRIPTION_LEN: usize = 200;

/// Function values a variable or class field can name (`const f = () => {}`).
const FUNCTION_VALUE_KINDS: &[&str] = &["arrow_function", "function_expression", "function"];

struct TypeScriptCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    language: Language,
    test_file: bool,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    /// Named functions enclosing the current position, innermost last.
    functions: Vec<NodeId>,
    /// Depth of statically dead branches around the walk position.
    dead: usize,
}

impl<'a> TypeScriptCollector<'a> {
    fn new(content: &'a str, file_path: &'a str, language: Language, test_file: bool) -> Self {
        Self {
            content,
            file_path,
            language,
            test_file,
            nodes: Vec::new(),
            edges: Vec::new(),
            functions: Vec::new(),
            dead: 0,
        }
    }

    fn span_for(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
    }

    fn into_result(self) -> ExtractionResult {
        ExtractionResult {
            nodes: self.nodes,
            edges: self.edges,
        }
    }

    fn walk(&mut self, node: Node) {
        if self.dead > 0 {
            return;
        }
        let kind = node.kind();
        if FUNCTION_KINDS.contains(&kind) {
            if let Some(name) = self.field_text(&node, "name") {
                return self.function(name, node, node);
            }
        } else if matches!(kind, "variable_declarator" | "public_field_definition") {
            let value = node.child_by_field_name("value");
            if let (Some(name), Some(value)) = (self.field_text(&node, "name"), value)
                && FUNCTION_VALUE_KINDS.contains(&value.kind())
            {
                // CRG spans a variable function over its whole declaration statement.
                let extent = match node.parent() {
                    Some(parent) if kind == "variable_declarator" => parent,
                    _ => node,
                };
                return self.function(name, extent, node);
            }
        } else if kind == "assignment_expression" {
            let value = node.child_by_field_name("right");
            if let (Some(left), Some(value)) = (node.child_by_field_name("left"), value)
                && left.kind() == "member_expression"
                && FUNCTION_VALUE_KINDS.contains(&value.kind())
            {
                let extent = node
                    .parent()
                    .filter(|parent| parent.kind() == "expression_statement")
                    .unwrap_or(node);
                return self.function(self.node_text(&left), extent, value);
            }
        } else if CLASS_KINDS.contains(&kind) {
            if let Some(name) = self.field_text(&node, "name") {
                let node_type = if kind == "interface_declaration" {
                    NodeType::Interface
                } else {
                    NodeType::Class
                };
                let code = self.code_node(name, node_type, &node);
                for base in self.bases(&node) {
                    self.edges.push(EdgeRelationship {
                        from: code.id,
                        to: base,
                        edge_type: EdgeType::Extends,
                        metadata: HashMap::from([(
                            "source_file".to_string(),
                            self.file_path.to_string(),
                        )]),
                        span: Some(self.span_for(&node)),
                    });
                }
                self.nodes.push(code);
            }
        } else if kind == "if_statement" {
            return self.if_statement(&node);
        } else if kind == "import_statement" {
            self.import(&node);
        } else if kind == "call_expression" {
            if let Some((specifier, import_type)) = self.module_import(&node) {
                self.module_import_edge(&node, specifier, import_type);
                self.walk_children(node);
                return;
            }
            if let Some(runner) = self.test_runner(&node) {
                // The callback's calls belong to the test, not to a CALLS edge to `it`.
                self.functions.push(runner);
                self.walk_children(node);
                self.functions.pop();
                return;
            }
            self.call(&node);
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child);
        }
    }

    fn walk_dead(&mut self, node: Node, dead: bool) {
        self.dead += usize::from(dead);
        self.walk(node);
        self.dead -= usize::from(dead);
    }

    /// CRG's unreachable-call visitor drops the consequence of a statically false `if`
    /// (including declarations nested there). `else`/`elif` branches stay live.
    fn if_statement(&mut self, node: &Node) {
        let dead = node
            .child_by_field_name("condition")
            .is_some_and(|condition| self.static_false(&condition));
        let consequence = node.child_by_field_name("consequence");
        let alternative = node.child_by_field_name("alternative");
        if let Some(body) = consequence {
            self.walk_dead(body, dead);
        }
        if let Some(body) = alternative {
            self.walk(body);
        }
        // Non-branch children (the condition itself) stay live.
        let ids = [consequence.map(|n| n.id()), alternative.map(|n| n.id())];
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if !ids.contains(&Some(child.id())) {
                self.walk(child);
            }
        }
    }

    fn static_false(&self, node: &Node) -> bool {
        let node = if node.kind() == "parenthesized_expression" {
            let Some(inner) = node.named_child(0) else {
                return false;
            };
            inner
        } else {
            *node
        };
        node.kind() == "false" || (node.kind() == "number" && self.node_text(&node) == "0")
    }

    /// Records a named function spanning `extent` and walks `body` inside it.
    fn function(&mut self, name: String, extent: Node, body: Node) {
        let code = self
            .code_node(name, NodeType::Function, &extent)
            .with_complexity(crate::complexity::calculate_cyclomatic_complexity(
                &body,
                self.content,
            ));
        self.functions.push(code.id);
        self.nodes.push(code);
        self.walk_children(body);
        self.functions.pop();
    }

    fn code_node(&self, name: String, node_type: NodeType, node: &Node) -> CodeNode {
        let mut code = CodeNode::new(
            name,
            Some(node_type),
            Some(self.language.clone()),
            self.location(node),
        )
        .with_content(self.node_text(node));
        code.span = Some(self.span_for(node));
        code
    }

    fn import(&mut self, node: &Node) {
        let Some(source) = node.child_by_field_name("source") else {
            return;
        };
        let name = self
            .node_text(&source)
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        let code = self.code_node(name.clone(), NodeType::Import, node);
        self.edges.push(EdgeRelationship {
            from: code.id,
            to: name,
            edge_type: EdgeType::Imports,
            metadata: HashMap::from([
                ("import_type".to_string(), "es_import".to_string()),
                ("source_file".to_string(), self.file_path.to_string()),
            ]),
            span: Some(self.span_for(node)),
        });
        self.nodes.push(code);
    }

    /// `require('./x')` and `import('./x')` are module imports, not calls to `require`.
    fn module_import(&self, node: &Node) -> Option<(String, &'static str)> {
        if self.dead > 0 {
            return None;
        }
        let callee = node.child_by_field_name("function")?;
        let import_type = match callee.kind() {
            "import" => "dynamic_import",
            "identifier" if self.node_text(&callee) == "require" => "require",
            _ => return None,
        };
        let arguments = node.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        let argument = arguments
            .children(&mut cursor)
            .find(|a| matches!(a.kind(), "string" | "template_string"))?;
        let specifier = self
            .node_text(&argument)
            .trim_matches(|c| matches!(c, '\'' | '"' | '`'))
            .to_string();
        (!specifier.is_empty()).then_some((specifier, import_type))
    }

    fn module_import_edge(&mut self, node: &Node, specifier: String, import_type: &str) {
        self.edges.push(EdgeRelationship {
            from: NodeId::nil(),
            to: specifier,
            edge_type: EdgeType::Imports,
            metadata: HashMap::from([
                ("import_type".to_string(), import_type.to_string()),
                ("source_file".to_string(), self.file_path.to_string()),
                (crg_shape::FILE_SCOPE_ATTR.to_string(), "true".to_string()),
            ]),
            span: Some(self.span_for(node)),
        });
    }

    fn call(&mut self, node: &Node) {
        if self.dead > 0 {
            return;
        }
        let Some(target) = self.call_name(node) else {
            return;
        };
        let mut metadata = HashMap::from([
            ("call_type".to_string(), "function_call".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        // Outside every function the call belongs to the file, which `crg_shape` adds.
        let from = match self.functions.last() {
            Some(&id) => id,
            None => {
                metadata.insert(crg_shape::FILE_SCOPE_ATTR.to_string(), "true".to_string());
                NodeId::nil()
            }
        };
        self.edges.push(EdgeRelationship {
            from,
            to: target,
            edge_type: EdgeType::Calls,
            metadata,
            span: Some(self.span_for(node)),
        });
    }

    /// In a test file, a call to a test runner (`describe('S', ...)`, `it.skip(...)`) becomes a
    /// test node named `<runner>:<description>@L<line>`, like CRG's `_extract_calls`.
    fn test_runner(&mut self, node: &Node) -> Option<NodeId> {
        if !self.test_file {
            return None;
        }
        let runner = self.runner_name(node)?;
        let line = node.start_position().row + 1;
        let name = match self.test_description(node) {
            Some(description) => format!("{runner}:{description}@L{line}"),
            None => format!("{runner}@L{line}"),
        };
        let mut code = self.code_node(name, NodeType::Function, node);
        code.metadata
            .attributes
            .insert(crg_shape::TEST_RUNNER_ATTR.into(), "true".into());
        let id = code.id;
        self.nodes.push(code);
        Some(id)
    }

    /// The runner a call names: `it`, or the base of a modifier call (`it.skip` -> `it`).
    fn runner_name(&self, node: &Node) -> Option<String> {
        let name = self.call_name(node)?;
        if test_detection::is_test_runner_name(&name) {
            return Some(name);
        }
        if !TEST_MODIFIER_SUFFIXES.contains(&name.as_str()) {
            return None;
        }
        let callee = node.child_by_field_name("function")?;
        let mut object = callee.child_by_field_name("object")?;
        while object.kind() == "member_expression" {
            object = object.child_by_field_name("object")?;
        }
        let base = (object.kind() == "identifier").then(|| self.node_text(&object))?;
        test_detection::is_test_runner_name(&base).then_some(base)
    }

    /// CRG `_get_test_description`: the first string argument, unquoted, whitespace collapsed
    /// and cut to `MAX_TEST_DESCRIPTION_LEN` characters.
    fn test_description(&self, node: &Node) -> Option<String> {
        let arguments = node.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        let argument = arguments
            .children(&mut cursor)
            .find(|a| matches!(a.kind(), "string" | "template_string"))?;
        let text = self.node_text(&argument);
        let stripped = text.trim_matches(|c| matches!(c, '\'' | '"' | '`'));
        let normalized = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
        let description: String = normalized.chars().take(MAX_TEST_DESCRIPTION_LEN).collect();
        (!description.is_empty()).then_some(description)
    }

    /// CRG `_get_bases`: the names in `extends`/`implements` clauses (both become CRG
    /// `INHERITS`); of a generic base, its head. Other base forms (`ns.J`) name nothing.
    fn bases(&self, node: &Node) -> Vec<String> {
        let mut clauses = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_heritage" {
                let mut inner = child.walk();
                clauses.extend(child.children(&mut inner));
            } else {
                clauses.push(child);
            }
        }
        let mut bases = Vec::new();
        for clause in clauses {
            if !matches!(
                clause.kind(),
                "extends_clause" | "implements_clause" | "extends_type_clause"
            ) {
                continue;
            }
            let mut cursor = clause.walk();
            for sub in clause.children(&mut cursor) {
                match sub.kind() {
                    "identifier" | "type_identifier" | "nested_identifier" => {
                        bases.push(self.node_text(&sub))
                    }
                    "generic_type" => {
                        let mut inner = sub.walk();
                        if let Some(head) = sub.children(&mut inner).find(|n| {
                            matches!(n.kind(), "type_identifier" | "nested_type_identifier")
                        }) {
                            bases.push(self.node_text(&head));
                        }
                    }
                    _ => {}
                }
            }
        }
        bases
    }

    /// CRG `_get_call_name`: the callee identifier, or the rightmost member of a member
    /// expression (`res.status(404).json` -> `json`); other callees (`super(...)`,
    /// `import(...)`, `f()()`, `(0, f)()`) name nothing.
    fn call_name(&self, node: &Node) -> Option<String> {
        let callee = node.child_by_field_name("function")?;
        match callee.kind() {
            "identifier" => Some(self.node_text(&callee)),
            "member_expression" => match callee.child_by_field_name("property") {
                Some(property) if property.kind() == "property_identifier" => {
                    Some(self.node_text(&property))
                }
                _ => Some(self.node_text(&callee)),
            },
            _ => None,
        }
    }

    fn field_text(&self, node: &Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|child| self.node_text(&child))
    }

    fn location(&self, node: &Node) -> Location {
        Location {
            file_path: self.file_path.to_string(),
            line: node.start_position().row as u32 + 1,
            column: node.start_position().column as u32,
            end_line: Some(node.end_position().row as u32 + 1),
            end_column: Some(node.end_position().column as u32),
        }
    }

    fn node_text(&self, node: &Node) -> String {
        node.utf8_text(self.content.as_bytes())
            .unwrap_or("")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn extract_ts(source: &str, path: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        crate::languages::extract_for_language_at(
            &Language::TypeScript,
            &tree,
            source,
            path,
            path,
            Default::default(),
        )
        .unwrap()
    }

    fn qualified_names(result: &ExtractionResult) -> BTreeSet<String> {
        result
            .nodes
            .iter()
            .filter_map(|n| n.metadata.attributes.get("qualified_name").cloned())
            .collect()
    }

    fn calls_from(result: &ExtractionResult, qualified: &str) -> Vec<String> {
        let Some(node) = result.nodes.iter().find(|n| {
            n.metadata
                .attributes
                .get("qualified_name")
                .map(String::as_str)
                == Some(qualified)
        }) else {
            return Vec::new();
        };
        let mut targets: Vec<String> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls && e.from == node.id)
            .map(|e| e.to.clone())
            .collect();
        targets.sort();
        targets
    }

    #[test]
    fn class_methods_interfaces_and_types_are_nodes() {
        let r = extract_ts(
            "interface D { id: number }\ntype T = string;\nenum E { A }\n\
             abstract class Base { abstract run(): void; }\n\
             class S extends Base {\n  get(id: number) { return id; }\n  handler = () => go();\n}\n\
             export const f = (a: number) => a;\nfunction g() {}\n",
            "m.ts",
        );
        let qns = qualified_names(&r);
        for q in [
            "m.ts::D",
            "m.ts::T",
            "m.ts::E",
            "m.ts::Base",
            "m.ts::Base.run",
            "m.ts::S",
            "m.ts::S.get",
            "m.ts::S.handler",
            "m.ts::f",
            "m.ts::g",
        ] {
            assert!(qns.contains(q), "{q} missing from {qns:?}");
        }
    }

    #[test]
    fn anonymous_arrows_are_not_nodes() {
        let r = extract_ts("function outer() {\n  [1].map(x => inner(x));\n}\n", "m.ts");
        let qns = qualified_names(&r);
        assert!(!qns.iter().any(|q| q.ends_with("::x")), "{qns:?}");
        assert!(calls_from(&r, "m.ts::outer").contains(&"inner".to_string()));
    }

    #[test]
    fn call_names_are_the_rightmost_member() {
        let r = extract_ts(
            "function h(res) {\n  res.status(404).json({});\n  this.users.get(1);\n  Date.now();\n\
             Number(1);\n  new Map();\n  super.x();\n  (0, f)();\n}\n",
            "m.ts",
        );
        assert_eq!(
            calls_from(&r, "m.ts::h"),
            vec!["Number", "get", "json", "now", "status", "x"]
        );
    }

    #[test]
    fn module_scope_calls_leave_the_file() {
        let r = extract_ts("function a() {}\nmain();\n", "m.ts");
        assert!(calls_from(&r, "m.ts::a").is_empty());
        assert_eq!(calls_from(&r, "m.ts"), vec!["main"]);
        assert!(r.edges.iter().all(|e| {
            !e.metadata
                .contains_key(super::super::crg_shape::FILE_SCOPE_ATTR)
        }));
    }

    fn node<'r>(result: &'r ExtractionResult, qualified: &str) -> &'r CodeNode {
        result
            .nodes
            .iter()
            .find(|n| {
                n.metadata
                    .attributes
                    .get("qualified_name")
                    .map(String::as_str)
                    == Some(qualified)
            })
            .unwrap_or_else(|| panic!("{qualified} missing"))
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

    #[test]
    fn runner_calls_in_test_files_become_tests() {
        let src = "describe.only('S', () => {\n  it.skip(`finds  one`, () => { svc.findById(1); });\n\
                   beforeEach(() => {});\n});\n";
        let r = extract_ts(src, "a.test.ts");
        let qns = qualified_names(&r);
        for q in [
            "a.test.ts::describe:S@L1",
            "a.test.ts::it:finds one@L2",
            "a.test.ts::beforeEach@L3",
        ] {
            assert!(qns.contains(q), "{q} missing from {qns:?}");
        }
        let edges = contains_edges(&r);
        assert!(edges.contains(&("a.test.ts".into(), "a.test.ts::describe:S@L1".into())));
        assert!(edges.contains(&(
            "a.test.ts::describe:S@L1".into(),
            "a.test.ts::it:finds one@L2".into()
        )));
        assert_eq!(
            calls_from(&r, "a.test.ts::it:finds one@L2"),
            vec!["findById"]
        );
        assert!(calls_from(&r, "a.test.ts").is_empty());
        let it = node(&r, "a.test.ts::it:finds one@L2");
        assert_eq!(it.node_type, Some(NodeType::Other("Test".into())));
        assert_eq!(it.metadata.attributes["is_test"], "true");
        assert_eq!(crate::languages::crg_shape::id_node_type(it), "Function");
        assert_eq!((it.location.line, it.location.end_line), (2, Some(2)));
    }

    #[test]
    fn runner_calls_outside_test_files_are_plain_calls() {
        let r = extract_ts("describe('S', () => { go(); });\n", "src/a.ts");
        assert!(!qualified_names(&r).iter().any(|q| q.contains('@')));
        assert_eq!(calls_from(&r, "src/a.ts"), vec!["describe", "go"]);
    }

    #[test]
    fn runner_descriptions_are_capped_at_200_characters() {
        let long = "x".repeat(250);
        let r = extract_ts(&format!("it('{long}', () => {{}});\n"), "a.test.ts");
        let expected = format!("a.test.ts::it:{}@L1", "x".repeat(200));
        assert!(qualified_names(&r).contains(&expected));
    }

    #[test]
    fn commonjs_and_dynamic_imports_become_import_edges() {
        let r = extract_ts(
            "const a = require('./util');\nconst b = import('./other');\n",
            "m.js",
        );
        let mut imports: Vec<_> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Imports)
            .map(|e| e.to.clone())
            .collect();
        imports.sort();
        assert_eq!(imports, ["./other", "./util"]);
        assert!(
            !r.edges
                .iter()
                .any(|e| e.edge_type == EdgeType::Calls && e.to == "require")
        );
    }

    #[test]
    fn js_dead_guards_drop_calls_in_the_false_branch() {
        let r = extract_ts(
            "function f() {\n  if (false) { dead(); } else { live(); }\n}\n",
            "m.ts",
        );
        assert_eq!(calls_from(&r, "m.ts::f"), vec!["live"]);
    }

    #[test]
    fn js_dead_guards_drop_nested_declarations_but_keep_the_else_branch() {
        let r = extract_ts(
            "function f() {\n  if (0) { function nested() { deep(); } } else { live(); }\n}\n",
            "m.ts",
        );
        assert_eq!(calls_from(&r, "m.ts::f"), vec!["live"]);
        assert!(!qualified_names(&r).iter().any(|q| q.contains("nested")));
    }

    #[test]
    fn assigned_functions_and_prototype_methods_are_nodes() {
        let r = extract_ts(
            "exports.run = function run() { helper(); };\n\
             Thing.prototype.go = function go() { helper(); };\n",
            "m.js",
        );
        let qns = qualified_names(&r);
        assert!(qns.contains("m.js::exports.run"), "{qns:?}");
        assert!(qns.contains("m.js::Thing.prototype.go"), "{qns:?}");
        assert_eq!(calls_from(&r, "m.js::exports.run"), vec!["helper"]);
        assert_eq!(calls_from(&r, "m.js::Thing.prototype.go"), vec!["helper"]);
    }

    #[test]
    fn heritage_clauses_become_extends_edges() {
        let r = extract_ts(
            "class A extends B<T> implements I, ns.J {}\ninterface K extends L, M {}\n",
            "m.ts",
        );
        let qn_of = |id: NodeId| {
            r.nodes
                .iter()
                .find(|n| n.id == id)
                .unwrap()
                .metadata
                .attributes["qualified_name"]
                .clone()
        };
        let mut got: Vec<(String, String)> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Extends)
            .map(|e| (qn_of(e.from), e.to.clone()))
            .collect();
        got.sort();
        let expected: Vec<(String, String)> = [
            ("m.ts::A", "B"),
            ("m.ts::A", "I"),
            ("m.ts::K", "L"),
            ("m.ts::K", "M"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(got, expected);
    }
}
