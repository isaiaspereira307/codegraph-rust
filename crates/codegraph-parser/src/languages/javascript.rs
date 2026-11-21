// ABOUTME: TypeScript/JavaScript extractor producing code-review-graph's declarations and calls.
// ABOUTME: Tracks the enclosing named function so calls in anonymous callbacks stay with it.

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

/// Function values a variable or class field can name (`const f = () => {}`).
const FUNCTION_VALUE_KINDS: &[&str] = &["arrow_function", "function_expression", "function"];

struct TypeScriptCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    language: Language,
    #[allow(dead_code)]
    test_file: bool,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    /// Named functions enclosing the current position, innermost last.
    functions: Vec<NodeId>,
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
        } else if CLASS_KINDS.contains(&kind) {
            if let Some(name) = self.field_text(&node, "name") {
                let node_type = if kind == "interface_declaration" {
                    NodeType::Interface
                } else {
                    NodeType::Class
                };
                let code = self.code_node(name, node_type, &node);
                self.nodes.push(code);
            }
        } else if kind == "import_statement" {
            self.import(&node);
        } else if kind == "call_expression" {
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

    fn call(&mut self, node: &Node) {
        let Some(&from) = self.functions.last() else {
            return;
        };
        let Some(target) = self.extract_call_target(node) else {
            return;
        };
        self.edges.push(EdgeRelationship {
            from,
            to: target,
            edge_type: EdgeType::Calls,
            metadata: HashMap::from([
                ("call_type".to_string(), "function_call".to_string()),
                ("source_file".to_string(), self.file_path.to_string()),
            ]),
            span: Some(self.span_for(node)),
        });
    }

    fn extract_call_target(&self, node: &Node) -> Option<String> {
        node.child_by_field_name("function")
            .map(|function| self.node_text(&function))
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
}
