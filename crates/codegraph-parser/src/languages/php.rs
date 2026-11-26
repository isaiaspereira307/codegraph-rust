// ABOUTME: PHP extractor shaped like code-review-graph: classes/interfaces/traits/enums as Class,
// ABOUTME: `use`/include imports, call names and constructed-receiver scopes for resolution.

use super::crg_shape;
use super::test_detection;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::HashMap;
use tree_sitter::{Node, Tree};

/// Set on a PHP call edge: the class scope its receiver or `Scope::` names, as code-review-graph
/// records it (`receiver_scope`, or the resolved `Class::method` scope). The indexer binds the
/// call to that class's method only with file or namespace evidence.
pub const PHP_SCOPE_ATTR: &str = "php_scope";
/// Set with [`PHP_SCOPE_ATTR`] when the scope is `self`/`static`: only the enclosing class in
/// the caller's own file may answer.
pub const PHP_SAME_FILE_ATTR: &str = "php_same_file";
/// Set on PHP class and function nodes: the namespace declared around them.
pub const PHP_NAMESPACE_ATTR: &str = "php_namespace";
/// Set on PHP `use` import edges that import a function or constant, not a class.
pub const PHP_USE_KIND_ATTR: &str = "php_use_kind";

/// PHP extractor.
///
/// Extracts classes, interfaces, traits and enums (all `Class`, like code-review-graph),
/// functions and methods (PHPUnit `test*` methods in test files, `@test` docblocks and
/// `#[Test]` attributes become tests), `use` and literal include imports, and calls:
/// functions, methods, `Scope::method` and `new Class`.
pub struct PhpExtractor;

#[derive(Default, Clone)]
struct PhpContext {
    current_class: Option<String>,
    current_function: Option<NodeId>,
    is_framework_file: bool,
}

impl PhpExtractor {
    pub fn extract(tree: &Tree, content: &str, file_path: &str) -> Vec<CodeNode> {
        Self::extract_with_edges(tree, content, file_path).nodes
    }

    /// Unified extraction of nodes AND edges in single AST traversal
    pub fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        let mut collector = PhpCollector::new(content, file_path);
        let ctx = PhpContext {
            is_framework_file: file_path.contains("/app/")
                || file_path.contains("/src/")
                || file_path.contains("Controller.php")
                || file_path.contains("Model.php"),
            ..PhpContext::default()
        };
        collector.walk(tree.root_node(), &ctx);
        ExtractionResult {
            nodes: collector.nodes,
            edges: collector.edges,
        }
    }
}

impl super::LanguageExtractor for PhpExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        PhpExtractor::extract_with_edges(tree, content, file_path)
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[
            EdgeType::Imports,
            EdgeType::Calls,
            EdgeType::Implements,
            EdgeType::Extends,
        ]
    }

    fn language() -> Language {
        Language::Php
    }
}

struct PhpCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    is_test_file: bool,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    /// Namespace in effect (`namespace A\B;` applies to the statements after it).
    namespace: String,
    /// Class `use` aliases of the current namespace: case-folded local name → qualified name.
    imports: HashMap<String, String>,
    /// `$variable` → the class text of the `new` last assigned to it in this scope.
    bindings: HashMap<String, String>,
}

/// CRG `_php_resolve_class_reference`: an absolute name as written, an imported head through
/// its alias, otherwise relative to the current namespace.
fn resolve_class_reference(
    reference: &str,
    namespace: &str,
    imports: &HashMap<String, String>,
) -> String {
    let normalized = reference.trim_matches('\\');
    if reference.starts_with('\\') {
        return normalized.to_string();
    }
    let (head, tail) = match normalized.split_once('\\') {
        Some((head, tail)) => (head, Some(tail)),
        None => (normalized, None),
    };
    if let Some(imported) = imports.get(&head.to_lowercase()) {
        return match tail {
            Some(tail) => format!("{imported}\\{tail}"),
            None => imported.clone(),
        };
    }
    if namespace.is_empty() {
        normalized.to_string()
    } else {
        format!("{namespace}\\{normalized}")
    }
}

impl<'a> PhpCollector<'a> {
    fn new(content: &'a str, file_path: &'a str) -> Self {
        Self {
            content,
            file_path,
            is_test_file: test_detection::is_test_file(&crg_shape::default_rel_path(file_path)),
            nodes: Vec::new(),
            edges: Vec::new(),
            namespace: String::new(),
            imports: HashMap::new(),
            bindings: HashMap::new(),
        }
    }

    fn span_for(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
    }

    fn walk_children(&mut self, node: Node, ctx: &PhpContext) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in children {
            self.walk(child, ctx);
        }
    }

    fn walk(&mut self, node: Node, ctx: &PhpContext) {
        match node.kind() {
            "namespace_definition" => {
                let name = self
                    .child_text_by_field(node, "name")
                    .map(|name| name.trim_matches('\\').to_string())
                    .unwrap_or_default();
                match node.child_by_field_name("body") {
                    Some(body) => {
                        let saved_namespace = std::mem::replace(&mut self.namespace, name);
                        let saved_imports = std::mem::take(&mut self.imports);
                        self.walk(body, ctx);
                        self.namespace = saved_namespace;
                        self.imports = saved_imports;
                    }
                    None => {
                        self.namespace = name;
                        self.imports.clear();
                    }
                }
            }

            "namespace_use_declaration" => self.use_declaration(node),

            "include_expression"
            | "include_once_expression"
            | "require_expression"
            | "require_once_expression" => {
                self.include(node);
                self.walk_children(node, ctx);
            }

            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => {
                let Some(name) = self.child_text_by_field(node, "name") else {
                    return self.walk_children(node, ctx);
                };
                self.class_node(node, &name, ctx);
                let inner = PhpContext {
                    current_class: Some(name),
                    current_function: None,
                    ..ctx.clone()
                };
                let saved = std::mem::take(&mut self.bindings);
                self.walk_children(node, &inner);
                self.bindings = saved;
            }

            "function_definition" | "method_declaration" => {
                let Some(name) = self.child_text_by_field(node, "name") else {
                    return self.walk_children(node, ctx);
                };
                let id = self.function_node(node, &name, ctx);
                let inner = PhpContext {
                    current_function: Some(id),
                    ..ctx.clone()
                };
                let saved = self.bindings.clone();
                self.walk_children(node, &inner);
                self.bindings = saved;
            }

            "compound_statement" => {
                let saved = self.bindings.clone();
                self.walk_children(node, ctx);
                self.bindings = saved;
            }

            "assignment_expression" => {
                // The initializer runs before the new binding is visible.
                self.walk_children(node, ctx);
                let Some(left) = node.child_by_field_name("left") else {
                    return;
                };
                if left.kind() != "variable_name" {
                    return;
                }
                let variable = self.node_text(&left);
                self.bindings.remove(&variable);
                if let Some(right) = node.child_by_field_name("right")
                    && right.kind() == "object_creation_expression"
                    && let Some(class) = self.creation_class(right)
                {
                    let class = class.trim_matches('\\').to_string();
                    if !class.is_empty() {
                        self.bindings.insert(variable, class);
                    }
                }
            }

            "function_call_expression"
            | "member_call_expression"
            | "nullsafe_member_call_expression"
            | "scoped_call_expression"
            | "object_creation_expression" => {
                self.call(node, ctx);
                self.walk_children(node, ctx);
            }

            _ => self.walk_children(node, ctx),
        }
    }

    fn class_node(&mut self, node: Node, name: &str, ctx: &PhpContext) {
        let content_text = self.node_text(&node);
        let mut code = CodeNode::new(
            name.to_string(),
            Some(NodeType::Class),
            Some(Language::Php),
            self.location(&node),
        )
        .with_content(content_text.clone());
        let attributes = &mut code.metadata.attributes;
        let kind = node.kind().trim_end_matches("_declaration");
        attributes.insert("kind".into(), kind.into());
        attributes.insert("php_kind".into(), kind.into());
        if !self.namespace.is_empty() {
            attributes.insert(PHP_NAMESPACE_ATTR.into(), self.namespace.clone());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "base_clause" => {
                    attributes.insert("inheritance".into(), self.node_text(&child));
                }
                "class_interface_clause" => {
                    attributes.insert("interfaces".into(), self.node_text(&child));
                }
                _ => {}
            }
        }
        if ctx.is_framework_file && node.kind() == "class_declaration" {
            let pattern = if name.ends_with("Controller") {
                Some("controller")
            } else if content_text.contains("extends Model") {
                Some("model")
            } else if content_text.contains("extends Migration") {
                Some("migration")
            } else {
                None
            };
            if let Some(pattern) = pattern {
                attributes.insert("laravel_pattern".into(), pattern.into());
            }
        }
        self.nodes.push(code);
    }

    fn function_node(&mut self, node: Node, name: &str, ctx: &PhpContext) -> NodeId {
        let content_text = self.node_text(&node);
        let mut code = CodeNode::new(
            name.to_string(),
            Some(NodeType::Function),
            Some(Language::Php),
            self.location(&node),
        )
        .with_content(content_text.clone())
        .with_complexity(crate::complexity::calculate_cyclomatic_complexity(
            &node,
            self.content,
        ));
        code.span = Some(self.span_for(&node));
        let modifiers: Vec<String> = {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .filter(|child| {
                    matches!(
                        child.kind(),
                        "visibility_modifier" | "static_modifier" | "abstract_modifier"
                    )
                })
                .map(|child| self.node_text(&child))
                .collect()
        };
        let decorators = self.test_decorators(node, name);
        let attributes = &mut code.metadata.attributes;
        attributes.insert("kind".into(), "function".into());
        if let Some(visibility) = modifiers
            .iter()
            .find(|m| matches!(m.as_str(), "public" | "protected" | "private"))
        {
            attributes.insert("visibility".into(), visibility.clone());
        }
        if modifiers.iter().any(|m| m == "static") {
            attributes.insert("static".into(), "true".into());
        }
        if name.starts_with("__") {
            attributes.insert("magic_method".into(), "true".into());
        }
        if let Some(return_type) = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(&n))
        {
            attributes.insert("return_type".into(), return_type);
        }
        if let Some(class) = &ctx.current_class {
            attributes.insert("parent_class".into(), class.clone());
        }
        if !self.namespace.is_empty() {
            attributes.insert(PHP_NAMESPACE_ATTR.into(), self.namespace.clone());
        }
        if !decorators.is_empty() {
            attributes.insert(crg_shape::DECORATORS_ATTR.into(), decorators.join(","));
        }
        let id = code.id;
        self.nodes.push(code);
        id
    }

    /// CRG's PHP test marks: `#[Test]` attributes (PHPUnit's attribute class counts), a
    /// `/** @test */` docblock right before the method, and the PHPUnit `test*` name
    /// convention for methods in test files. Each is recorded as the `Test` decorator.
    fn test_decorators(&self, node: Node, name: &str) -> Vec<String> {
        let mut decorators = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "attribute_list" {
                self.attribute_names(child, &mut decorators);
            }
        }
        if let Some(previous) = node.prev_sibling()
            && previous.kind() == "comment"
            && docblock_marks_test(&self.node_text(&previous))
        {
            decorators.push("Test".into());
        }
        if node.kind() == "method_declaration" && self.is_test_file && name.starts_with("test") {
            decorators.push("Test".into());
        }
        decorators
    }

    fn attribute_names(&self, list: Node, out: &mut Vec<String>) {
        let mut groups = list.walk();
        for group in list.children(&mut groups) {
            if group.kind() != "attribute_group" {
                continue;
            }
            let mut attributes = group.walk();
            for attribute in group.children(&mut attributes) {
                if attribute.kind() != "attribute" {
                    continue;
                }
                let mut parts = attribute.walk();
                let name = attribute
                    .children(&mut parts)
                    .find(|part| matches!(part.kind(), "name" | "qualified_name"))
                    .map(|part| self.node_text(&part));
                if let Some(name) = name {
                    let normalized = name.trim_start_matches('\\');
                    if normalized.eq_ignore_ascii_case("PHPUnit\\Framework\\Attributes\\Test") {
                        out.push("Test".into());
                    } else {
                        out.push(normalized.to_string());
                    }
                }
            }
        }
    }

    /// `use A\B;`, `use A\B as C;`, `use A\{B, C as D};`, `use function a\b;`: one import per
    /// clause, naming the fully qualified symbol (the alias only binds the local name).
    fn use_declaration(&mut self, node: Node) {
        let statement = self.node_text(&node);
        let lowered = statement.trim_start().to_lowercase();
        let symbol_kind = if lowered.starts_with("use function ") {
            Some("function")
        } else if lowered.starts_with("use const ") {
            Some("const")
        } else {
            None
        };
        let mut prefix = String::new();
        let mut clauses = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "namespace_name" => prefix = self.node_text(&child).trim_matches('\\').to_string(),
                "namespace_use_clause" => clauses.push(child),
                "namespace_use_group" => {
                    let mut group = child.walk();
                    clauses.extend(
                        child
                            .children(&mut group)
                            .filter(|clause| clause.kind() == "namespace_use_clause"),
                    );
                }
                _ => {}
            }
        }
        let mut targets = Vec::new();
        for clause in clauses {
            let alias_node = clause.child_by_field_name("alias");
            let alias = alias_node.map(|alias| self.node_text(&alias));
            let mut parts = clause.walk();
            let imported = clause
                .children(&mut parts)
                .find(|part| {
                    matches!(part.kind(), "name" | "qualified_name") && Some(*part) != alias_node
                })
                .map(|part| self.node_text(&part).trim_matches('\\').to_string());
            let Some(imported) = imported.filter(|i| !i.is_empty()) else {
                continue;
            };
            let qualified = if prefix.is_empty() {
                imported
            } else {
                format!("{prefix}\\{imported}")
            };
            if symbol_kind.is_none() {
                let local = alias
                    .clone()
                    .unwrap_or_else(|| qualified.rsplit('\\').next().unwrap_or("").to_string());
                self.imports.insert(local.to_lowercase(), qualified.clone());
            }
            targets.push(qualified);
        }
        if targets.is_empty() {
            return;
        }
        let mut code = CodeNode::new(
            statement.clone(),
            Some(NodeType::Import),
            Some(Language::Php),
            self.location(&node),
        )
        .with_content(statement);
        code.span = Some(self.span_for(&node));
        code.metadata.attributes.insert("kind".into(), "use".into());
        for target in targets {
            let mut metadata = HashMap::from([
                ("import_type".to_string(), "php_use".to_string()),
                ("source_file".to_string(), self.file_path.to_string()),
            ]);
            if let Some(kind) = symbol_kind {
                metadata.insert(PHP_USE_KIND_ATTR.into(), kind.into());
            }
            self.edges.push(EdgeRelationship {
                from: code.id,
                to: target,
                edge_type: EdgeType::Imports,
                metadata,
                span: Some(self.span_for(&node)),
            });
        }
        self.nodes.push(code);
    }

    /// `include`/`require` with a literal path (a double-quoted one only without
    /// interpolation or escapes); computed paths such as `__DIR__ . '/x.php'` are skipped,
    /// as at code-review-graph.
    fn include(&mut self, node: Node) {
        let mut cursor = node.walk();
        let literal = node.children(&mut cursor).find_map(|child| {
            let plain = child.kind() == "string"
                || (child.kind() == "encapsed_string" && {
                    let mut parts = child.walk();
                    child
                        .named_children(&mut parts)
                        .all(|part| part.kind() == "string_content")
                });
            plain.then(|| {
                self.node_text(&child)
                    .trim_matches(|c| c == '\'' || c == '"')
                    .to_string()
            })
        });
        let Some(path) = literal.filter(|path| !path.is_empty()) else {
            return;
        };
        let statement = self.node_text(&node);
        let mut code = CodeNode::new(
            statement.clone(),
            Some(NodeType::Import),
            Some(Language::Php),
            self.location(&node),
        )
        .with_content(statement);
        code.span = Some(self.span_for(&node));
        code.metadata
            .attributes
            .insert("kind".into(), "include".into());
        self.edges.push(EdgeRelationship {
            from: code.id,
            to: path,
            edge_type: EdgeType::Imports,
            metadata: HashMap::from([
                ("import_type".to_string(), "php_include".to_string()),
                ("source_file".to_string(), self.file_path.to_string()),
            ]),
            span: Some(self.span_for(&node)),
        });
        self.nodes.push(code);
    }

    fn creation_class(&self, node: Node) -> Option<String> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|child| matches!(child.kind(), "name" | "qualified_name"))
            .map(|child| self.node_text(&child))
    }

    /// One CALLS edge, named like CRG's `_get_call_name`: the function, the method, the
    /// `Scope::method` or the instantiated class, without a leading `\`.
    fn call(&mut self, node: Node, ctx: &PhpContext) {
        let mut metadata = HashMap::from([("call_type".to_string(), "php_call".to_string())]);
        let target = match node.kind() {
            "function_call_expression" => node
                .child_by_field_name("function")
                .filter(|f| matches!(f.kind(), "name" | "qualified_name"))
                .map(|f| self.node_text(&f).trim_start_matches('\\').to_string()),
            "member_call_expression" | "nullsafe_member_call_expression" => {
                let method = node.child_by_field_name("name").map(|n| self.node_text(&n));
                if method.is_some()
                    && let Some(object) = node.child_by_field_name("object")
                    && object.kind() == "variable_name"
                    && let Some(class) = self.bindings.get(&self.node_text(&object))
                {
                    // CRG `_resolve_typed_method_target`: the local class name through the
                    // imports, otherwise the class as written.
                    let local = class.rsplit('\\').next().unwrap_or(class);
                    let scope = self
                        .imports
                        .get(&local.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| class.clone());
                    metadata.insert(PHP_SCOPE_ATTR.into(), scope);
                }
                method
            }
            "scoped_call_expression" => {
                let scope = node
                    .child_by_field_name("scope")
                    .map(|s| self.node_text(&s));
                let method = node.child_by_field_name("name").map(|n| self.node_text(&n));
                match (scope, method) {
                    (Some(scope), Some(method)) if !scope.is_empty() && !method.is_empty() => {
                        let lowered = scope.trim_matches('\\').to_lowercase();
                        if matches!(lowered.as_str(), "self" | "static") {
                            if let Some(class) = &ctx.current_class {
                                metadata.insert(PHP_SCOPE_ATTR.into(), class.clone());
                                metadata.insert(PHP_SAME_FILE_ATTR.into(), "true".into());
                            }
                        } else if lowered != "parent" && !scope.starts_with('$') {
                            metadata.insert(
                                PHP_SCOPE_ATTR.into(),
                                resolve_class_reference(&scope, &self.namespace, &self.imports),
                            );
                        }
                        metadata.insert(crg_shape::VERBATIM_TARGET_ATTR.into(), "true".into());
                        Some(format!("{}::{method}", scope.trim_start_matches('\\')))
                    }
                    _ => None,
                }
            }
            "object_creation_expression" => self
                .creation_class(node)
                .map(|class| class.trim_start_matches('\\').to_string()),
            _ => None,
        };
        let Some(target) = target.filter(|t| !t.is_empty()) else {
            return;
        };
        let from = match ctx.current_function {
            Some(id) => id,
            None => {
                metadata.insert(crg_shape::FILE_SCOPE_ATTR.into(), "true".into());
                NodeId::nil()
            }
        };
        self.edges.push(EdgeRelationship {
            from,
            to: target,
            edge_type: EdgeType::Calls,
            metadata,
            span: Some(self.span_for(&node)),
        });
    }

    fn child_text_by_field(&self, node: Node, field_name: &str) -> Option<String> {
        node.child_by_field_name(field_name)
            .map(|child| self.node_text(&child))
    }

    fn node_text(&self, node: &Node) -> String {
        node.utf8_text(self.content.as_bytes())
            .unwrap_or("")
            .to_string()
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

/// CRG `_PHP_TEST_DOC_TAG_RE`: an `@test` tag not glued to a word or `-`.
fn docblock_marks_test(comment: &str) -> bool {
    let bytes = comment.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    comment.match_indices("@test").any(|(start, _)| {
        let end = start + "@test".len();
        (start == 0 || !word(bytes[start - 1])) && (end >= bytes.len() || !word(bytes[end]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExtractionPolicy;
    use crate::languages::extract_for_language_at;
    use tree_sitter::Parser;

    fn extract(path: &str, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_for_language_at(
            &Language::Php,
            &tree,
            source,
            path,
            path,
            ExtractionPolicy::default(),
        )
        .unwrap()
    }

    fn qualified(result: &ExtractionResult) -> Vec<(String, String)> {
        result
            .nodes
            .iter()
            .filter_map(|n| {
                Some((
                    format!("{:?}", n.node_type.as_ref()?),
                    n.metadata.attributes.get("qualified_name")?.clone(),
                ))
            })
            .collect()
    }

    #[test]
    fn php_namespaces_are_not_nodes_and_type_likes_are_classes() {
        let result = extract(
            "src/App.php",
            "<?php\nnamespace Demo;\ninterface I {}\ntrait T {}\nenum E {}\nclass App { private $x; public function run() {} }\n",
        );
        let names = qualified(&result);
        for class in ["I", "T", "E", "App"] {
            assert!(
                names.contains(&("Class".into(), format!("src/App.php::{class}"))),
                "{class}: {names:?}"
            );
        }
        assert!(names.contains(&("Function".into(), "src/App.php::App.run".into())));
        assert!(!names.iter().any(|(_, q)| q.contains("Demo")), "{names:?}");
    }

    #[test]
    fn php_use_clauses_import_fully_qualified_names() {
        let result = extract(
            "src/App.php",
            "<?php\nnamespace A;\nuse \\B\\C;\nuse D\\{E, F as G};\nuse function h\\i;\nrequire_once 'x.php';\nrequire_once __DIR__ . '/y.php';\n",
        );
        let mut imports: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Imports)
            .map(|e| e.to.as_str())
            .collect();
        imports.sort();
        assert_eq!(imports, ["B\\C", "D\\E", "D\\F", "h\\i", "x.php"]);
    }

    #[test]
    fn php_calls_record_names_and_receiver_scopes() {
        let result = extract(
            "src/App.php",
            "<?php\nnamespace Demo;\nuse Lib\\Repo;\nclass App {\n  function run() {\n    $r = new Repo();\n    $r->save();\n    $o = new \\Other\\Thing();\n    $o->go();\n    $o = 1;\n    $o->stop();\n    Helper::make();\n    self::local();\n    \\strlen('a');\n  }\n}\n",
        );
        let calls: HashMap<&str, Option<&String>> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .map(|e| (e.to.as_str(), e.metadata.get(PHP_SCOPE_ATTR)))
            .collect();
        assert_eq!(calls["Repo"], None);
        assert_eq!(calls["save"].map(String::as_str), Some("Lib\\Repo"));
        assert_eq!(calls["Other\\Thing"], None);
        assert_eq!(calls["go"].map(String::as_str), Some("Other\\Thing"));
        assert_eq!(calls["stop"], None);
        assert_eq!(
            calls["Helper::make"].map(String::as_str),
            Some("Demo\\Helper")
        );
        assert_eq!(calls["self::local"].map(String::as_str), Some("App"));
        assert!(calls.contains_key("strlen"));
    }

    #[test]
    fn phpunit_conventions_mark_tests() {
        let result = extract(
            "tests/AppTest.php",
            "<?php\nclass AppTest {\n  public function testRun() {}\n  /** @test */\n  public function buildsFresh() {}\n  #[Test]\n  public function viaAttribute() {}\n  /** @testing */\n  public function helper() {}\n}\n",
        );
        let names = qualified(&result);
        for test in ["testRun", "buildsFresh", "viaAttribute"] {
            assert!(
                names.contains(&(
                    "Other(\"Test\")".into(),
                    format!("tests/AppTest.php::AppTest.{test}")
                )),
                "{test}: {names:?}"
            );
        }
        assert!(names.contains(&(
            "Function".into(),
            "tests/AppTest.php::AppTest.helper".into()
        )));
        // Outside a test file, the `test*` name alone is not a PHPUnit test.
        let result = extract(
            "src/App.php",
            "<?php\nclass App { function testRun() {} }\n",
        );
        assert!(
            qualified(&result).contains(&("Function".into(), "src/App.php::App.testRun".into()))
        );
    }
}
