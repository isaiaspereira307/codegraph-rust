// ABOUTME: Go language AST extractor for code intelligence
// ABOUTME: Extracts packages, functions, methods, types, imports, embedding and call edges

use super::crg_shape;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Tree};

/// Go AST extractor in code-review-graph's shape.
///
/// Extracts:
/// - packages, functions, methods, types (every `type_spec`; aliases are not nodes)
/// - imports (single and grouped)
/// - calls by their final name (`fmt.Printf` -> `Printf`), leaving the enclosing named
///   function, or the file outside every function; calls under `if false` are dropped
/// - `recv.M()` on the method's own, unshadowed receiver as `<file>::Type.M`
/// - struct and interface embedding as `Extends` text
pub struct GoExtractor;

impl GoExtractor {
    pub fn extract(tree: &Tree, content: &str, file_path: &str) -> Vec<CodeNode> {
        Self::extract_with_edges(tree, content, file_path).nodes
    }

    /// Unified extraction of nodes AND edges in single AST traversal
    pub fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        let mut collector = GoCollector::new(content, file_path);
        collector.count_methods(tree.root_node());
        collector.walk(tree.root_node());
        ExtractionResult {
            nodes: collector.nodes,
            edges: collector.edges,
        }
    }
}

impl super::LanguageExtractor for GoExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        GoExtractor::extract_with_edges(tree, content, file_path)
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[EdgeType::Imports, EdgeType::Calls, EdgeType::Extends]
    }

    fn language() -> Language {
        Language::Go
    }
}

/// Lexical scopes of CRG's `_go_receiver_binding_index`.
const SCOPE_KINDS: &[&str] = &[
    "block",
    "communication_case",
    "default_case",
    "expression_case",
    "type_case",
];

/// The receiver of the method being walked.
struct Receiver {
    name: String,
    /// The receiver's type (`*List[T]` -> `List`).
    owner: String,
    /// Merged byte ranges `[start, end)` where a local binding hides the receiver.
    shadows: Vec<(usize, usize)>,
}

struct GoCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    /// Named functions around the walk position, innermost last.
    functions: Vec<NodeId>,
    receiver: Option<Receiver>,
    /// Methods per `(receiver type, name)` in this file.
    methods: HashMap<(String, String), usize>,
    /// Depth of `if false` consequences around the walk position.
    dead: usize,
}

impl<'a> GoCollector<'a> {
    fn new(content: &'a str, file_path: &'a str) -> Self {
        Self {
            content,
            file_path,
            nodes: Vec::new(),
            edges: Vec::new(),
            functions: Vec::new(),
            receiver: None,
            methods: HashMap::new(),
            dead: 0,
        }
    }

    fn count_methods(&mut self, root: Node) {
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if child.kind() != "method_declaration" {
                continue;
            }
            if let (Some(owner), Some(name)) = (
                receiver_type(&child).map(|t| self.node_text(&t)),
                self.field_text(&child, "name"),
            ) {
                *self.methods.entry((owner, name)).or_default() += 1;
            }
        }
    }

    fn span_for(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "package_clause" => self.package(&node),
            "import_spec" => self.import_spec(&node),
            "function_declaration" | "method_declaration" => return self.function(node),
            "type_spec" => self.type_spec(&node),
            "if_statement" => return self.if_statement(node),
            "call_expression" => self.call(&node),
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child);
        }
    }

    /// Calls in the consequence of `if false` (or `if (false)`) are dead, like CRG's
    /// `_is_in_static_dead_guard`; `else` branches stay live.
    fn if_statement(&mut self, node: Node) {
        let dead_branch = node
            .child_by_field_name("condition")
            .filter(is_statically_false)
            .and_then(|_| node.child_by_field_name("consequence"));
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let dead = dead_branch.is_some_and(|branch| branch.id() == child.id());
            self.dead += usize::from(dead);
            self.walk(child);
            self.dead -= usize::from(dead);
        }
    }

    fn package(&mut self, node: &Node) {
        let Some(name) = self.field_text(node, "name") else {
            return;
        };
        let mut code = CodeNode::new(
            name,
            Some(NodeType::Module),
            Some(Language::Go),
            self.location(node),
        )
        .with_content(self.node_text(node));
        code.span = Some(self.span_for(node));
        code.metadata
            .attributes
            .insert("kind".into(), "package".into());
        self.nodes.push(code);
    }

    fn function(&mut self, node: Node) {
        let Some(name) = self.field_text(&node, "name") else {
            return self.walk_children(node);
        };
        let is_method = node.kind() == "method_declaration";
        let mut code = CodeNode::new(
            name.clone(),
            Some(NodeType::Function),
            Some(Language::Go),
            self.location(&node),
        )
        .with_content(self.node_text(&node))
        .with_complexity(crate::complexity::calculate_cyclomatic_complexity(
            &node,
            self.content,
        ));
        code.span = Some(self.span_for(&node));
        let attributes = &mut code.metadata.attributes;
        if !is_method && name == "main" {
            attributes.insert("entry_point".into(), "true".into());
        }
        if !is_method && name == "init" {
            attributes.insert("init_function".into(), "true".into());
        }
        if name.chars().next().is_some_and(char::is_uppercase) {
            attributes.insert("exported".into(), "true".into());
        }
        let kind = if is_method { "method" } else { "function" };
        attributes.insert("kind".into(), kind.into());
        if let Some(receiver) = node.child_by_field_name("receiver") {
            attributes.insert("receiver".into(), self.node_text(&receiver));
        }

        let receiver = if is_method {
            self.receiver_of(&node)
        } else {
            None
        };
        let outer = std::mem::replace(&mut self.receiver, receiver);
        self.functions.push(code.id);
        self.nodes.push(code);
        self.walk_children(node);
        self.functions.pop();
        self.receiver = outer;
    }

    /// The method's named receiver and where the body hides it.
    fn receiver_of(&self, method: &Node) -> Option<Receiver> {
        let owner = self.node_text(&receiver_type(method)?);
        let parameter = method
            .child_by_field_name("receiver")
            .and_then(|r| named_children(&r).find(|c| c.kind() == "parameter_declaration"))?;
        let name = children(&parameter).find(|c| c.kind() == "identifier")?;
        let name = self.node_text(&name);
        let shadows = method
            .child_by_field_name("body")
            .map(|body| self.shadow_intervals(body, &name))
            .unwrap_or_default();
        Some(Receiver {
            name,
            owner,
            shadows,
        })
    }

    /// CRG's `_go_receiver_binding_index`: byte ranges where a local declaration of `name`
    /// hides the receiver, merged.
    fn shadow_intervals(&self, body: Node, name: &str) -> Vec<(usize, usize)> {
        let mut intervals = Vec::new();
        let mut scopes = HashSet::from([scope_key(&body)]);
        let mut stack = vec![(body, body)];
        while let Some((node, scope)) = stack.pop() {
            let scope = if SCOPE_KINDS.contains(&node.kind()) {
                node
            } else {
                scope
            };
            match node.kind() {
                "func_literal" => {
                    if let Some(literal_body) = node.child_by_field_name("body")
                        && self.parameters_bind(&node, name)
                    {
                        intervals.push((literal_body.start_byte(), literal_body.end_byte()));
                        scopes.insert(scope_key(&literal_body));
                    }
                }
                "type_switch_statement" if self.field_binds(&node, "alias", name) => {
                    for case in named_children(&node)
                        .filter(|c| matches!(c.kind(), "default_case" | "type_case"))
                    {
                        if let Some(statements) =
                            named_children(&case).find(|c| c.kind() == "statement_list")
                        {
                            intervals.push((statements.start_byte(), case.end_byte()));
                        }
                        scopes.insert(scope_key(&case));
                    }
                }
                "const_spec" | "var_spec" | "type_spec" | "type_alias"
                    if self.field_binds(&node, "name", name) =>
                {
                    intervals.push((node.end_byte(), scope.end_byte()));
                    scopes.insert(scope_key(&scope));
                }
                "short_var_declaration" if self.field_binds(&node, "left", name) => {
                    let binding_scope = match node.parent() {
                        Some(parent) if parent.kind() == "for_clause" => parent.parent(),
                        Some(parent)
                            if matches!(
                                parent.kind(),
                                "expression_switch_statement"
                                    | "if_statement"
                                    | "type_switch_statement"
                            ) =>
                        {
                            Some(parent)
                        }
                        _ => Some(scope),
                    };
                    if let Some(binding_scope) = binding_scope
                        && scopes.insert(scope_key(&binding_scope))
                    {
                        intervals.push((node.end_byte(), binding_scope.end_byte()));
                    }
                }
                "range_clause"
                    if has_child(&node, ":=") && self.field_binds(&node, "left", name) =>
                {
                    if let Some(loop_body) =
                        node.parent().and_then(|l| l.child_by_field_name("body"))
                    {
                        intervals.push((loop_body.start_byte(), loop_body.end_byte()));
                        scopes.insert(scope_key(&loop_body));
                    }
                }
                "receive_statement"
                    if has_child(&node, ":=") && self.field_binds(&node, "left", name) =>
                {
                    if let Some(case) = node.parent() {
                        intervals.push((node.end_byte(), case.end_byte()));
                        scopes.insert(scope_key(&case));
                    }
                }
                _ => {}
            }
            let children: Vec<_> = named_children(&node).collect();
            stack.extend(children.into_iter().rev().map(|child| (child, scope)));
        }

        intervals.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in intervals {
            if start >= end {
                continue;
            }
            match merged.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        merged
    }

    /// Whether a grammar field of `node` declares `name` (CRG `_go_field_binds_name`).
    fn field_binds(&self, node: &Node, field: &str, name: &str) -> bool {
        let mut cursor = node.walk();
        node.children_by_field_name(field, &mut cursor)
            .any(|field_node| {
                let candidates: Vec<Node> = if field_node.kind() == "expression_list" {
                    named_children(&field_node).collect()
                } else {
                    vec![field_node]
                };
                candidates.iter().any(|c| {
                    matches!(c.kind(), "identifier" | "type_identifier")
                        && self.node_text(c) == name
                })
            })
    }

    /// Whether a func literal's parameters or named results declare `name`.
    fn parameters_bind(&self, function: &Node, name: &str) -> bool {
        ["parameters", "result"].iter().any(|field| {
            let mut cursor = function.walk();
            function
                .children_by_field_name(field, &mut cursor)
                .any(|list| {
                    named_children(&list).any(|parameter| {
                        matches!(
                            parameter.kind(),
                            "parameter_declaration" | "variadic_parameter_declaration"
                        ) && self.field_binds(&parameter, "name", name)
                    })
                })
        })
    }

    fn import_spec(&mut self, node: &Node) {
        let Some(path_node) = node.child_by_field_name("path") else {
            return;
        };
        let clean_path = self.node_text(&path_node).trim_matches('"').to_string();
        if clean_path.is_empty() {
            return;
        }

        let mut code = CodeNode::new(
            format!("import {}", clean_path),
            Some(NodeType::Import),
            Some(Language::Go),
            self.location(node),
        )
        .with_content(self.node_text(node));
        code.span = Some(self.span_for(node));

        let alias = self.field_text(node, "name");
        if let Some(alias) = &alias {
            code.metadata
                .attributes
                .insert("alias".into(), alias.clone());
        }
        let is_stdlib = !clean_path.contains('.');
        if is_stdlib {
            code.metadata
                .attributes
                .insert("stdlib".into(), "true".into());
        }
        code.metadata
            .attributes
            .insert("kind".into(), "import".into());
        code.metadata
            .attributes
            .insert("path".into(), clean_path.clone());

        let mut metadata = HashMap::from([
            ("import_type".to_string(), "go_import".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        if is_stdlib {
            metadata.insert("stdlib".to_string(), "true".to_string());
        }
        if let Some(alias) = alias {
            metadata.insert("alias".into(), alias);
        }
        self.edges.push(EdgeRelationship {
            from: code.id,
            to: clean_path,
            edge_type: EdgeType::Imports,
            metadata,
            span: Some(self.span_for(node)),
        });
        self.nodes.push(code);
    }

    /// Every `type_spec` is a class in CRG: structs, interfaces and defined types
    /// (`type C int`). `type A = B` is a `type_alias` and makes no node.
    fn type_spec(&mut self, node: &Node) {
        let Some(name) = self.field_text(node, "name") else {
            return;
        };
        let body = node.child_by_field_name("type");
        let (node_type, kind) = match body.map(|b| b.kind()) {
            Some("interface_type") => (NodeType::Interface, "interface"),
            Some("struct_type") => (NodeType::Struct, "struct"),
            _ => (NodeType::Class, "defined_type"),
        };

        let mut code = CodeNode::new(
            name.clone(),
            Some(node_type),
            Some(Language::Go),
            self.location(node),
        )
        .with_content(self.node_text(node));
        code.span = Some(self.span_for(node));
        if name.chars().next().is_some_and(char::is_uppercase) {
            code.metadata
                .attributes
                .insert("exported".into(), "true".into());
        }
        code.metadata.attributes.insert("kind".into(), kind.into());

        for base in body.map(|b| self.embedded(&b)).unwrap_or_default() {
            self.edges.push(EdgeRelationship {
                from: code.id,
                to: base,
                edge_type: EdgeType::Extends,
                metadata: HashMap::from([("source_file".to_string(), self.file_path.to_string())]),
                span: Some(self.span_for(node)),
            });
        }
        self.nodes.push(code);
    }

    /// Embedded struct fields and single-type interface elements, as CRG's `_get_bases`.
    fn embedded(&self, body: &Node) -> Vec<String> {
        let embedded: Vec<Node> = match body.kind() {
            "struct_type" => named_children(body)
                .filter(|c| c.kind() == "field_declaration_list")
                .flat_map(|list| named_children(&list).collect::<Vec<_>>())
                .filter(|f| {
                    f.kind() == "field_declaration" && f.child_by_field_name("name").is_none()
                })
                .filter_map(|f| f.child_by_field_name("type"))
                .collect(),
            "interface_type" => named_children(body)
                .filter(|c| c.kind() == "type_elem")
                .filter_map(|element| {
                    let types: Vec<Node> = named_children(&element)
                        .filter(|c| c.kind() != "comment")
                        .collect();
                    (types.len() == 1).then(|| types[0])
                })
                .collect(),
            _ => Vec::new(),
        };
        embedded
            .into_iter()
            .filter_map(|t| {
                if t.kind() == "generic_type" {
                    t.child_by_field_name("type")
                } else {
                    Some(t)
                }
            })
            .filter(|t| matches!(t.kind(), "type_identifier" | "qualified_type"))
            .map(|t| self.node_text(&t))
            .collect()
    }

    fn call(&mut self, node: &Node) {
        if self.dead > 0 {
            return;
        }
        let Some(callee) = node.child_by_field_name("function") else {
            return;
        };
        let mut qualifier = None;
        let target = match callee.kind() {
            "identifier" => self.node_text(&callee),
            "selector_expression" => match callee.child_by_field_name("field") {
                Some(field) => {
                    let name = self.node_text(&field);
                    let operand = callee
                        .child_by_field_name("operand")
                        .filter(|operand| operand.kind() == "identifier")
                        .map(|operand| self.node_text(&operand));
                    if operand
                        .as_ref()
                        .is_some_and(|text| self.receiver.as_ref().is_none_or(|r| r.name != *text))
                    {
                        qualifier = operand;
                    }
                    self.resolve_on_receiver(node, &callee, name)
                }
                None => return,
            },
            // Func literals, parenthesized and index callees, calls of calls: no edge.
            _ => return,
        };

        let mut metadata = HashMap::from([
            ("call_type".to_string(), "go_call".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        if let Some(qualifier) = qualifier {
            metadata.insert("go_package_qualifier".to_string(), qualifier);
        }
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

    /// `recv.M()` on the method's own receiver, outside every shadowing binding, names
    /// `<file>::Type.M` when the receiver type has exactly one `M` in this file; any other
    /// selector call keeps the bare method name.
    fn resolve_on_receiver(&self, call: &Node, callee: &Node, method: String) -> String {
        let Some(receiver) = &self.receiver else {
            return method;
        };
        let Some(mut operand) = callee.child_by_field_name("operand") else {
            return method;
        };
        while operand.kind() == "parenthesized_expression" && operand.named_child_count() == 1 {
            operand = operand.named_child(0).unwrap();
        }
        if operand.kind() == "unary_expression"
            && operand.child(0).is_some_and(|c| c.kind() == "*")
            && let Some(inner) = operand.child_by_field_name("operand")
        {
            operand = inner;
        }
        let start = call.start_byte();
        let shadowed = receiver
            .shadows
            .iter()
            .any(|&(s, e)| s <= start && start < e);
        let unique = self.methods.get(&(receiver.owner.clone(), method.clone())) == Some(&1);
        if self.node_text(&operand) == receiver.name && !shadowed && unique {
            format!("{}::{}.{method}", self.file_path, receiver.owner)
        } else {
            method
        }
    }

    fn field_text(&self, node: &Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|n| self.node_text(&n))
            .filter(|text| !text.is_empty())
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

/// The `type_identifier` a method's receiver names, through `*`, parentheses and type
/// arguments (CRG `_get_go_receiver_type`).
fn receiver_type<'t>(method: &Node<'t>) -> Option<Node<'t>> {
    let parameter = method
        .child_by_field_name("receiver")
        .and_then(|r| named_children(&r).find(|c| c.kind() == "parameter_declaration"))?;
    let mut ty = parameter.child_by_field_name("type");
    while let Some(t) = ty
        && matches!(
            t.kind(),
            "generic_type" | "pointer_type" | "parenthesized_type"
        )
    {
        ty = named_children(&t).find(|c| c.kind() != "comment");
    }
    ty.filter(|t| t.kind() == "type_identifier")
}

/// `false`, possibly parenthesized (CRG `_is_statically_false_condition`).
fn is_statically_false(condition: &Node) -> bool {
    match condition.kind() {
        "parenthesized_expression" => condition
            .named_child(0)
            .is_some_and(|inner| is_statically_false(&inner)),
        "false" => true,
        _ => false,
    }
}

fn scope_key(node: &Node) -> (u16, usize, usize) {
    (node.kind_id(), node.start_byte(), node.end_byte())
}

fn has_child(node: &Node, kind: &str) -> bool {
    children(node).any(|c| c.kind() == kind)
}

fn children<'t>(node: &Node<'t>) -> impl Iterator<Item = Node<'t>> {
    let node = *node;
    (0..node.child_count()).filter_map(move |i| node.child(i))
}

fn named_children<'t>(node: &Node<'t>) -> impl Iterator<Item = Node<'t>> {
    let node = *node;
    (0..node.named_child_count() as u32).filter_map(move |i| node.named_child(i))
}

#[cfg(test)]
mod tests {
    use codegraph_core::{EdgeType, ExtractionResult, Language, NodeType};
    use std::collections::BTreeSet;

    fn extract_go(source: &str, path: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        crate::languages::extract_for_language_at(
            &Language::Go,
            &tree,
            source,
            path,
            path,
            Default::default(),
        )
        .unwrap()
    }

    fn qualified(result: &ExtractionResult, id: &codegraph_core::NodeId) -> String {
        result
            .nodes
            .iter()
            .find(|n| &n.id == id)
            .and_then(|n| n.metadata.attributes.get("qualified_name").cloned())
            .unwrap_or_default()
    }

    /// `(source, target)` of every edge of `kind`.
    fn edges(result: &ExtractionResult, kind: EdgeType) -> BTreeSet<(String, String)> {
        result
            .edges
            .iter()
            .filter(|e| e.edge_type == kind)
            .map(|e| (qualified(result, &e.from), e.to.clone()))
            .collect()
    }

    fn calls_from(result: &ExtractionResult, source: &str) -> BTreeSet<String> {
        edges(result, EdgeType::Calls)
            .into_iter()
            .filter(|(from, _)| from == source)
            .map(|(_, to)| to)
            .collect()
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_type_spec_is_a_class_and_aliases_are_not_nodes() {
        let r = extract_go(
            "package p\ntype C int\ntype A = C\ntype S struct{}\ntype I interface{}\n\
             func (c C) M() {}\n",
            "m.go",
        );
        let kinds: Vec<_> = r
            .nodes
            .iter()
            .filter_map(|n| {
                let q = n.metadata.attributes.get("qualified_name")?;
                Some((q.clone(), n.node_type.clone()))
            })
            .collect();
        assert!(kinds.contains(&("m.go::C".to_string(), Some(NodeType::Class))));
        assert!(kinds.contains(&("m.go::S".to_string(), Some(NodeType::Struct))));
        assert!(kinds.contains(&("m.go::I".to_string(), Some(NodeType::Interface))));
        assert!(!kinds.iter().any(|(q, _)| q == "m.go::A"), "{kinds:?}");
        assert!(edges(&r, EdgeType::Contains).contains(&("m.go::C".into(), "m.go::C.M".into())));
    }

    #[test]
    fn call_names_and_callers_follow_crg() {
        let r = extract_go(
            "package p\nvar x = mk()\nfunc f(r *T) {\n  fmt.Printf(\"\")\n  (*r).Save()\n\
             func() { inner() }()\n  fs[0]()\n  (g)()\n  h()()\n  Generic[int](1)\n}\n",
            "m.go",
        );
        assert_eq!(
            calls_from(&r, "m.go::f"),
            set(&["Printf", "Save", "h", "inner"])
        );
        assert_eq!(calls_from(&r, "m.go"), set(&["mk"]));
    }

    #[test]
    fn selector_calls_keep_a_package_qualifier_without_changing_the_target() {
        let r = extract_go("package p\nfunc f() { helper.Save(); x.Load() }\n", "m.go");
        let calls: Vec<_> = edges(&r, EdgeType::Calls)
            .into_iter()
            .map(|(from, to)| (from, to))
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls.iter().map(|(_, to)| to.as_str()).collect::<Vec<_>>(),
            vec!["Load", "Save"]
        );
        let qualified = r
            .edges
            .iter()
            .filter(|edge| edge.edge_type == EdgeType::Calls)
            .filter_map(|edge| edge.metadata.get("go_package_qualifier"))
            .collect::<Vec<_>>();
        assert_eq!(qualified, vec!["helper", "x"]);
    }

    #[test]
    fn calls_under_if_false_are_dead() {
        let r = extract_go(
            "package p\nfunc f() {\n  if false {\n    dead()\n  } else {\n    live()\n  }\n\
             if (false) { dead2() }\n  if true { live2() }\n  if !true { live3() }\n}\n",
            "m.go",
        );
        assert_eq!(calls_from(&r, "m.go::f"), set(&["live", "live2", "live3"]));
    }

    #[test]
    fn receiver_calls_resolve_unless_shadowed() {
        let r = extract_go(
            "package p\ntype A struct{}\ntype B struct{}\nfunc (a *A) Save() {}\n\
             func (b *B) Save() {}\nfunc (a *A) Run() {}\nfunc (a *A) Run2() {}\n\
             func (a *A) Direct() {\n  a.Save()\n  (*a).Run()\n}\n\
             func (a *A) Shadowed() {\n  {\n    var a *B\n    a.Save()\n  }\n  a.Save()\n}\n\
             func (a *A) Literal() {\n  func(a *B) { a.Save() }(nil)\n}\n\
             func (a *A) Redeclared() {\n  a, n := a, 1\n  _ = n\n  a.Save()\n}\n\
             func (a *A) Range() {\n  for a := range []int{1} { a.Save() }\n}\n",
            "m.go",
        );
        assert_eq!(
            calls_from(&r, "m.go::A.Direct"),
            set(&["m.go::A.Run", "m.go::A.Save"])
        );
        assert_eq!(
            calls_from(&r, "m.go::A.Shadowed"),
            set(&["Save", "m.go::A.Save"])
        );
        assert_eq!(calls_from(&r, "m.go::A.Literal"), set(&["Save"]));
        assert_eq!(calls_from(&r, "m.go::A.Redeclared"), set(&["m.go::A.Save"]));
        assert_eq!(calls_from(&r, "m.go::A.Range"), set(&["Save"]));
    }

    #[test]
    fn embedding_is_inherits_text() {
        let r = extract_go(
            "package p\ntype D struct {\n  Base\n  *Ptr\n  pkg.Remote\n  G[int]\n  name Named\n}\n\
             type I interface {\n  io.Reader\n  int | string\n  M()\n}\n",
            "m.go",
        );
        let expected: BTreeSet<(String, String)> = [
            ("m.go::D", "Base"),
            ("m.go::D", "Ptr"),
            ("m.go::D", "pkg.Remote"),
            ("m.go::D", "G"),
            ("m.go::I", "io.Reader"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(edges(&r, EdgeType::Extends), expected);
    }
}
