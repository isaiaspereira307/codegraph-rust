// ABOUTME: Kotlin extractor shaped like code-review-graph: classes/objects, functions, imports,
// ABOUTME: final-name calls with typed receivers, supertypes as text and `@Test` annotations.

use super::crg_shape;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Tree};

/// Kotlin extractor.
///
/// Like code-review-graph: `class`/`interface`/`enum class`/`object` declarations are `Class`
/// nodes, `fun` declarations are functions (`@Test`-style annotations make them tests),
/// `import` headers name their module (`a.b.*` for wildcards), calls keep the final name and,
/// when the receiver is a parameter or a declared-type local, its type (`receiver_type`),
/// and supertypes are `INHERITS` text as written.
pub struct KotlinExtractor;

impl super::LanguageExtractor for KotlinExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        let mut collector = KotlinCollector {
            content,
            file_path,
            nodes: Vec::new(),
            edges: Vec::new(),
            bindings: HashMap::new(),
            defined: HashSet::new(),
            imported: HashSet::new(),
        };
        collector.collect_defined(tree.root_node());
        collector.walk(tree.root_node(), None);
        ExtractionResult {
            nodes: collector.nodes,
            edges: collector.edges,
        }
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[EdgeType::Imports, EdgeType::Calls, EdgeType::Extends]
    }

    fn language() -> Language {
        Language::Kotlin
    }
}

struct KotlinCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    /// Name → declared type of the parameters, class parameters and typed locals in scope.
    bindings: HashMap<String, String>,
    /// Class-like names declared in this file.
    defined: HashSet<String>,
    /// Local names the file's imports bind.
    imported: HashSet<String>,
}

/// CRG `_base_type_name`: the type without nullability, type arguments or qualifier.
fn base_type_name(text: &str) -> String {
    let text = text.trim().trim_end_matches('?');
    let text = text.split('<').next().unwrap_or(text).trim();
    text.to_string()
}

impl<'a> KotlinCollector<'a> {
    fn collect_defined(&mut self, node: Node) {
        if matches!(node.kind(), "class_declaration" | "object_declaration")
            && let Some(name) = node.child_by_field_name("name")
        {
            self.defined.insert(self.text(&name));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect_defined(child);
        }
    }

    fn walk_children(&mut self, node: Node, caller: Option<NodeId>) {
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        for child in children {
            self.walk(child, caller);
        }
    }

    fn walk(&mut self, node: Node, caller: Option<NodeId>) {
        match node.kind() {
            "import" => self.import(node),
            "class_declaration" | "object_declaration" => {
                let saved = self.bindings.clone();
                if node.child_by_field_name("name").is_some() {
                    self.class(node);
                }
                // Class parameters type the receivers used by every member.
                self.bind_parameters(node);
                // Members are not called by an enclosing function; their initializers belong
                // to the file, as at code-review-graph.
                self.walk_children(node, None);
                self.bindings = saved;
            }
            "function_declaration" => {
                let saved = self.bindings.clone();
                let id = self.function(node);
                self.bind_parameters(node);
                self.walk_children(node, id.or(caller));
                self.bindings = saved;
            }
            "block" | "statements" => {
                let saved = self.bindings.clone();
                self.walk_children(node, caller);
                self.bindings = saved;
            }
            "property_declaration" => {
                // The initializer is evaluated before the declared name is visible.
                self.walk_children(node, caller);
                let mut cursor = node.walk();
                for declaration in node.children(&mut cursor) {
                    if declaration.kind() != "variable_declaration" {
                        continue;
                    }
                    let (name, ty) = self.name_and_type(declaration);
                    if let Some(name) = name {
                        match ty {
                            Some(ty) => {
                                self.bindings.insert(name, ty);
                            }
                            None => {
                                self.bindings.remove(&name);
                            }
                        }
                    }
                }
            }
            "call_expression" => {
                self.call(node, caller);
                self.walk_children(node, caller);
            }
            _ => self.walk_children(node, caller),
        }
    }

    /// `(name, base type)` of a `parameter`, `class_parameter` or `variable_declaration`.
    fn name_and_type(&self, node: Node) -> (Option<String>, Option<String>) {
        let mut cursor = node.walk();
        let mut name = None;
        let mut ty = None;
        for child in node.children(&mut cursor) {
            match child.kind() {
                "identifier" if name.is_none() => name = Some(self.text(&child)),
                "user_type" | "nullable_type" if ty.is_none() => {
                    ty = Some(base_type_name(&self.text(&child)));
                }
                _ => {}
            }
        }
        (name, ty.filter(|t| !t.is_empty()))
    }

    /// Binds the declared parameters of a function or primary constructor.
    fn bind_parameters(&mut self, node: Node) {
        let mut cursor = node.walk();
        let mut stack: Vec<Node> = node
            .children(&mut cursor)
            .filter(|c| {
                matches!(
                    c.kind(),
                    "primary_constructor" | "function_value_parameters"
                )
            })
            .collect();
        while let Some(current) = stack.pop() {
            let mut cursor = current.walk();
            for child in current.children(&mut cursor) {
                match child.kind() {
                    "class_parameters" => stack.push(child),
                    "parameter" | "class_parameter" => {
                        if let (Some(name), Some(ty)) = self.name_and_type(child) {
                            self.bindings.insert(name, ty);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn annotations(&self, node: Node) -> Vec<String> {
        let mut annotations = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "modifiers" {
                continue;
            }
            let mut modifiers = child.walk();
            for annotation in child.children(&mut modifiers) {
                if annotation.kind() != "annotation" {
                    continue;
                }
                let mut parts = annotation.walk();
                let name = annotation
                    .children(&mut parts)
                    .find(|part| matches!(part.kind(), "user_type" | "constructor_invocation"))
                    .map(|part| {
                        let text = self.text(&part);
                        text.split('(').next().unwrap_or(&text).trim().to_string()
                    });
                if let Some(name) = name.filter(|n| !n.is_empty()) {
                    annotations.push(name);
                }
            }
        }
        annotations
    }

    fn class(&mut self, node: Node) {
        let Some(name) = node.child_by_field_name("name").map(|n| self.text(&n)) else {
            return;
        };
        let mut code = CodeNode::new(
            name,
            Some(NodeType::Class),
            Some(Language::Kotlin),
            self.location(&node),
        )
        .with_content(self.text(&node));
        code.span = Some(self.span(&node));
        let attributes = &mut code.metadata.attributes;
        attributes.insert(
            "kind".into(),
            node.kind().trim_end_matches("_declaration").into(),
        );
        let annotations = self.annotations(node);
        if !annotations.is_empty() {
            attributes.insert(crg_shape::DECORATORS_ATTR.into(), annotations.join(","));
        }
        let id = code.id;
        self.nodes.push(code);
        // CRG `_get_bases` for Kotlin: each delegation specifier, as written.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "delegation_specifiers" {
                continue;
            }
            let mut specifiers = child.walk();
            for specifier in child.children(&mut specifiers) {
                if specifier.kind() != "delegation_specifier" {
                    continue;
                }
                self.edges.push(EdgeRelationship {
                    from: id,
                    to: self.text(&specifier),
                    edge_type: EdgeType::Extends,
                    metadata: HashMap::from([(
                        "source_file".to_string(),
                        self.file_path.to_string(),
                    )]),
                    span: Some(self.span(&specifier)),
                });
            }
        }
    }

    fn function(&mut self, node: Node) -> Option<NodeId> {
        let name = node.child_by_field_name("name").map(|n| self.text(&n))?;
        let mut code = CodeNode::new(
            name,
            Some(NodeType::Function),
            Some(Language::Kotlin),
            self.location(&node),
        )
        .with_content(self.text(&node))
        .with_complexity(crate::complexity::calculate_cyclomatic_complexity(
            &node,
            self.content,
        ));
        code.span = Some(self.span(&node));
        code.metadata
            .attributes
            .insert("kind".into(), "function".into());
        let annotations = self.annotations(node);
        if !annotations.is_empty() {
            code.metadata
                .attributes
                .insert(crg_shape::DECORATORS_ATTR.into(), annotations.join(","));
        }
        let id = code.id;
        self.nodes.push(code);
        Some(id)
    }

    /// `import a.b.C`, `import a.b.C as D` (alias recorded), `import a.b.*`.
    fn import(&mut self, node: Node) {
        let mut cursor = node.walk();
        let mut module = None;
        let mut alias = None;
        let mut wildcard = false;
        for child in node.children(&mut cursor) {
            match child.kind() {
                "qualified_identifier" => module = Some(self.text(&child)),
                "identifier" if module.is_some() => alias = Some(self.text(&child)),
                "*" => wildcard = true,
                _ => {}
            }
        }
        if !wildcard {
            wildcard = self.text(&node).trim_end().ends_with(".*");
        }
        let Some(module) = module.filter(|m| !m.is_empty()) else {
            return;
        };
        let target = if wildcard {
            format!("{module}.*")
        } else {
            module
        };
        let statement = self.text(&node);
        let mut code = CodeNode::new(
            statement.clone(),
            Some(NodeType::Import),
            Some(Language::Kotlin),
            self.location(&node),
        )
        .with_content(statement);
        code.span = Some(self.span(&node));
        let mut metadata = HashMap::from([
            ("import_type".to_string(), "kotlin_import".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        if wildcard {
            metadata.insert("wildcard".into(), "true".into());
        }
        if !wildcard {
            let local = alias
                .clone()
                .unwrap_or_else(|| target.rsplit('.').next().unwrap_or("").to_string());
            self.imported.insert(local);
        }
        if let Some(alias) = alias {
            metadata.insert("alias".into(), alias);
        }
        self.edges.push(EdgeRelationship {
            from: code.id,
            to: target,
            edge_type: EdgeType::Imports,
            metadata,
            span: Some(self.span(&node)),
        });
        self.nodes.push(code);
    }

    /// The called name (`f()`, `a.b.f()`, `this.f()`, `T()`), with the receiver's declared type
    /// when the receiver is a bound identifier, as CRG's typed-receiver pass records it.
    fn call(&mut self, node: Node, caller: Option<NodeId>) {
        let Some(callee) = node.named_child(0) else {
            return;
        };
        let mut metadata = HashMap::from([("call_type".to_string(), "kotlin_call".to_string())]);
        let name = match callee.kind() {
            "identifier" => Some(self.text(&callee)),
            "navigation_expression" => {
                let count = callee.named_child_count();
                let method = callee
                    .named_child(count.saturating_sub(1) as u32)
                    .filter(|last| last.kind() == "identifier" && count > 1)
                    .map(|last| self.text(&last));
                if method.is_some()
                    && let Some(receiver) = callee.named_child(0)
                    && receiver.kind() == "identifier"
                {
                    let receiver = self.text(&receiver);
                    if let Some(ty) = self.bindings.get(&receiver) {
                        metadata.insert("receiver_type".into(), ty.clone());
                    } else if receiver.starts_with(char::is_uppercase)
                        && (self.defined.contains(&receiver) || self.imported.contains(&receiver))
                    {
                        // A class name receiver: an object/companion call on a known type.
                        metadata.insert("receiver_type".into(), receiver.clone());
                    }
                    metadata.insert("receiver".into(), receiver);
                }
                method
            }
            _ => None,
        };
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            return;
        };
        let from = match caller {
            Some(id) => id,
            None => {
                metadata.insert(crg_shape::FILE_SCOPE_ATTR.into(), "true".into());
                NodeId::nil()
            }
        };
        self.edges.push(EdgeRelationship {
            from,
            to: name,
            edge_type: EdgeType::Calls,
            metadata,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ExtractionPolicy;
    use crate::languages::extract_for_language_at;
    use tree_sitter::Parser;

    fn extract(path: &str, source: &str) -> ExtractionResult {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_for_language_at(
            &Language::Kotlin,
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
    fn kotlin_declarations_imports_and_supertypes() {
        let result = extract(
            "src/App.kt",
            "package demo\nimport a.b.C\nimport x.y.Z as W\nimport q.*\ninterface I\nclass App(val r: Repo) : Base(), I {\n  fun run() {}\n}\nobject O\nfun top() = 1\n",
        );
        let names = qualified(&result);
        for (kind, name) in [
            ("Class", "src/App.kt::I"),
            ("Class", "src/App.kt::App"),
            ("Class", "src/App.kt::O"),
            ("Function", "src/App.kt::App.run"),
            ("Function", "src/App.kt::top"),
        ] {
            assert!(
                names.contains(&(kind.into(), name.into())),
                "{name}: {names:?}"
            );
        }
        let targets = |kind: EdgeType| {
            let mut found: Vec<_> = result
                .edges
                .iter()
                .filter(|e| e.edge_type == kind)
                .map(|e| e.to.clone())
                .collect();
            found.sort();
            found
        };
        assert_eq!(targets(EdgeType::Imports), ["a.b.C", "q.*", "x.y.Z"]);
        assert_eq!(targets(EdgeType::Extends), ["Base()", "I"]);
    }

    #[test]
    fn kotlin_calls_keep_final_names_and_typed_receivers() {
        let result = extract(
            "src/App.kt",
            "class App(val repo: Repo) {\n  fun run(h: Helper?) {\n    val x: Store = Store()\n    val y = Store()\n    h?.twice(2)\n    x.save()\n    y.load()\n    repo.find()\n    a.b.c()\n    this.k()\n  }\n}\n",
        );
        let calls: HashMap<&str, Option<&String>> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .map(|e| (e.to.as_str(), e.metadata.get("receiver_type")))
            .collect();
        assert_eq!(calls["twice"].map(String::as_str), Some("Helper"));
        assert_eq!(calls["save"].map(String::as_str), Some("Store"));
        assert_eq!(calls["load"], None);
        assert_eq!(calls["find"].map(String::as_str), Some("Repo"));
        assert_eq!(calls["c"], None);
        assert_eq!(calls["k"], None);
        assert_eq!(calls["Store"], None);
    }

    #[test]
    fn kotlin_test_annotations_mark_tests() {
        let result = extract(
            "src/test/kotlin/AppTest.kt",
            "class AppTest {\n  @Test fun runs() {}\n  fun testRun() {}\n}\n",
        );
        let names = qualified(&result);
        assert!(names.contains(&(
            "Other(\"Test\")".into(),
            "src/test/kotlin/AppTest.kt::AppTest.runs".into()
        )));
        // Like CRG, a `test*` name without an annotation stays a function.
        assert!(names.contains(&(
            "Function".into(),
            "src/test/kotlin/AppTest.kt::AppTest.testRun".into()
        )));
    }
}
