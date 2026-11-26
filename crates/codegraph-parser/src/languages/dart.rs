// ABOUTME: Dart extractor shaped like code-review-graph: classes/mixins/enums, function
// ABOUTME: signatures as functions, import/export URIs, file-scope final-name calls, bases as text.

use super::crg_shape;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::HashMap;
use tree_sitter::{Node, Tree};

/// Dart extractor.
///
/// Mirrors what code-review-graph records with its Dart grammar, which differs from this
/// crate's (`tree-sitter-dart`, nielsenko):
/// - `class`, `mixin` and `enum` declarations are `Class` nodes;
/// - every `function_signature` is a function whose extent is the signature alone (CRG's
///   function node excludes the body), so getters, setters and constructors are not nodes;
/// - calls keep the final name (`f()`, `a.b.f()`, `this.f()`) and always come from the
///   `File`: CRG never sees a body inside its function node. `new`/`const` constructions and
///   cascades are not calls there either;
/// - `import`/`export` URIs are imports as written; superclass, mixins and interfaces are
///   `INHERITS` text.
pub struct DartExtractor;

impl super::LanguageExtractor for DartExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        let mut collector = DartCollector {
            content,
            file_path,
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        collector.walk(tree.root_node());
        ExtractionResult {
            nodes: collector.nodes,
            edges: collector.edges,
        }
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[EdgeType::Imports, EdgeType::Calls, EdgeType::Extends]
    }

    fn language() -> Language {
        Language::Dart
    }
}

struct DartCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
}

impl<'a> DartCollector<'a> {
    fn walk(&mut self, node: Node) {
        match node.kind() {
            "import_or_export" => self.import(node),
            "class_declaration" | "mixin_declaration" | "enum_declaration" => self.class(node),
            "function_signature" => self.function(node),
            "call_expression" => self.call(node),
            _ => {}
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in children {
            self.walk(child);
        }
    }

    fn class(&mut self, node: Node) {
        let Some(name) = node.child_by_field_name("name").map(|n| self.text(&n)) else {
            return;
        };
        let mut code = CodeNode::new(
            name,
            Some(NodeType::Class),
            Some(Language::Dart),
            self.location(&node),
        )
        .with_content(self.text(&node));
        code.span = Some(self.span(&node));
        code.metadata.attributes.insert(
            "kind".into(),
            node.kind().trim_end_matches("_declaration").into(),
        );
        let id = code.id;
        self.nodes.push(code);
        // CRG `_get_bases` for Dart: the superclass, its mixins and the interfaces.
        let mut bases = Vec::new();
        if let Some(superclass) = node.child_by_field_name("superclass") {
            let mut cursor = superclass.walk();
            for child in superclass.children(&mut cursor) {
                match child.kind() {
                    "type" => bases.extend(self.type_identifier(child)),
                    "mixins" => {
                        let mut mixins = child.walk();
                        for mixin in child.children(&mut mixins) {
                            if mixin.kind() == "type" {
                                bases.extend(self.type_identifier(mixin));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            let mut cursor = interfaces.walk();
            for child in interfaces.children(&mut cursor) {
                if child.kind() == "type" {
                    bases.extend(self.type_identifier(child));
                }
            }
        }
        for base in bases {
            self.edges.push(EdgeRelationship {
                from: id,
                to: base,
                edge_type: EdgeType::Extends,
                metadata: HashMap::from([("source_file".to_string(), self.file_path.to_string())]),
                span: Some(self.span(&node)),
            });
        }
    }

    fn type_identifier(&self, node: Node) -> Option<String> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|child| child.kind() == "type_identifier")
            .map(|child| self.text(&child))
    }

    fn function(&mut self, node: Node) {
        let Some(name) = node.child_by_field_name("name").map(|n| self.text(&n)) else {
            return;
        };
        let mut code = CodeNode::new(
            name,
            Some(NodeType::Function),
            Some(Language::Dart),
            self.location(&node),
        )
        .with_content(self.text(&node));
        code.span = Some(self.span(&node));
        code.metadata
            .attributes
            .insert("kind".into(), "function".into());
        self.nodes.push(code);
    }

    /// `import`/`export` with their URI as written.
    fn import(&mut self, node: Node) {
        let Some(uri) = find_string_literal(node).map(|literal| {
            self.text(&literal)
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string()
        }) else {
            return;
        };
        if uri.is_empty() {
            return;
        }
        let statement = self.text(&node);
        let mut code = CodeNode::new(
            statement.clone(),
            Some(NodeType::Import),
            Some(Language::Dart),
            self.location(&node),
        )
        .with_content(statement);
        code.span = Some(self.span(&node));
        self.edges.push(EdgeRelationship {
            from: code.id,
            to: uri,
            edge_type: EdgeType::Imports,
            metadata: HashMap::from([
                ("import_type".to_string(), "dart_import".to_string()),
                ("source_file".to_string(), self.file_path.to_string()),
            ]),
            span: Some(self.span(&node)),
        });
        self.nodes.push(code);
    }

    /// A call by its final name, always from the file scope (see the type docs).
    fn call(&mut self, node: Node) {
        let Some(function) = node.child_by_field_name("function") else {
            return;
        };
        let name = match function.kind() {
            "identifier" => Some(self.text(&function)),
            "member_expression" | "null_aware_member_expression" => function
                .child_by_field_name("property")
                .map(|property| self.text(&property)),
            _ => None,
        };
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            return;
        };
        self.edges.push(EdgeRelationship {
            from: NodeId::nil(),
            to: name,
            edge_type: EdgeType::Calls,
            metadata: HashMap::from([
                ("call_type".to_string(), "dart_call".to_string()),
                (crg_shape::FILE_SCOPE_ATTR.to_string(), "true".to_string()),
            ]),
            span: Some(self.span(&node)),
        });
    }

    fn text(&self, node: &Node) -> String {
        node.utf8_text(self.content.as_bytes())
            .unwrap_or("")
            .to_string()
    }

    fn span(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
    }

    fn location(&self, node: &Node) -> Location {
        Location {
            file_path: self.file_path.to_string(),
            line: (node.start_position().row + 1) as u32,
            column: (node.start_position().column + 1) as u32,
            end_line: Some((node.end_position().row + 1) as u32),
            end_column: Some((node.end_position().column + 1) as u32),
        }
    }
}

fn find_string_literal(node: Node) -> Option<Node> {
    if node.kind() == "string_literal" {
        return Some(node);
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    children.into_iter().find_map(find_string_literal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExtractionPolicy;
    use crate::languages::extract_for_language_at;
    use tree_sitter::Parser;

    fn extract(source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_dart::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_for_language_at(
            &Language::Dart,
            &tree,
            source,
            "lib/app.dart",
            "lib/app.dart",
            ExtractionPolicy::default(),
        )
        .unwrap()
    }

    const SOURCE: &str = "import 'dart:async';\nimport './a.dart' as a;\nexport 'b.dart';\nabstract class B extends A with M implements I {\n  B(this.x);\n  int get v => 1;\n  void abs();\n  int run(int q) {\n    void inner() {}\n    final h = new Helper();\n    h?.go();\n    this.k();\n    return x.y.z(1);\n  }\n}\nmixin M { void mm() {} }\nenum E { a, b }\nvoid main() => print('x');\n";

    #[test]
    fn dart_declarations_follow_crg() {
        let result = extract(SOURCE);
        let mut names: Vec<(String, String, u32, Option<u32>)> = result
            .nodes
            .iter()
            .filter_map(|n| {
                Some((
                    format!("{:?}", n.node_type.as_ref()?),
                    n.metadata.attributes.get("qualified_name")?.clone(),
                    n.location.line,
                    n.location.end_line,
                ))
            })
            .filter(|(kind, ..)| kind != "Import")
            .collect();
        names.sort();
        let expected = [
            ("Class", "lib/app.dart::B", 4, 15),
            ("Class", "lib/app.dart::E", 17, 17),
            ("Class", "lib/app.dart::M", 16, 16),
            ("Function", "lib/app.dart::B.abs", 7, 7),
            ("Function", "lib/app.dart::B.inner", 9, 9),
            ("Function", "lib/app.dart::B.run", 8, 8),
            ("Function", "lib/app.dart::M.mm", 16, 16),
            ("Function", "lib/app.dart::main", 18, 18),
        ];
        for (kind, name, start, end) in expected {
            assert!(
                names
                    .iter()
                    .any(|(k, n, s, e)| k == kind && n == name && *s == start && *e == Some(end)),
                "{name}: {names:?}"
            );
        }
        let edges = |kind: EdgeType| {
            let mut found: Vec<_> = result
                .edges
                .iter()
                .filter(|e| e.edge_type == kind)
                .map(|e| e.to.clone())
                .collect();
            found.sort();
            found
        };
        assert_eq!(
            edges(EdgeType::Imports),
            ["./a.dart", "b.dart", "dart:async"]
        );
        assert_eq!(edges(EdgeType::Extends), ["A", "I", "M"]);
        assert_eq!(edges(EdgeType::Calls), ["go", "k", "print", "z"]);
    }
}
