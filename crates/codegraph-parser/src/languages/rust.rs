use super::crg_shape;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Tree, TreeCursor};

/// Advanced Rust AST extractor using tree-sitter-rust.
///
/// Extracts:
/// - structs, traits, impls, functions, enums, modules
/// - tracks trait implementations, generic parameters, lifetimes
/// - builds dependency info for `use` statements (stored in node metadata)
/// - handles macros, async functions, unsafe blocks
///
/// Notes:
/// - We encode rich details in `CodeNode.metadata.attributes` to avoid API changes.
/// - Names are kept simple; qualified names and contexts are added as metadata.
pub struct RustExtractor;

#[derive(Default, Clone)]
struct WalkContext {
    module_path: Vec<String>,
    current_impl_for: Option<String>,
    current_impl_trait: Option<String>,
    /// The trait whose body is being walked: its signatures and default methods belong to it.
    current_trait: Option<String>,
    /// Bare name of the impl target or trait being walked (CRG `enclosing_class`).
    class_name: Option<String>,
}

impl RustExtractor {
    /// Extract only nodes for backward compatibility
    pub fn extract(tree: &Tree, content: &str, file_path: &str) -> Vec<CodeNode> {
        Self::extract_with_edges(tree, content, file_path).nodes
    }

    /// Extract BOTH nodes and edges in single AST traversal for maximum speed
    pub fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        Self::extract_with_policy(tree, content, file_path, Default::default())
    }

    pub fn extract_with_policy(
        tree: &Tree,
        content: &str,
        file_path: &str,
        policy: super::ExtractionPolicy,
    ) -> ExtractionResult {
        let mut collector = Collector::new(content, file_path);
        collector.references = policy.references;
        collector.defined_names = defined_names(tree.root_node(), content);
        collector.inline_module_items = inline_module_items(tree.root_node(), content);
        let mut cursor = tree.walk();
        collector.walk(&mut cursor, WalkContext::default());
        collector.into_result()
    }
}

impl super::LanguageExtractor for RustExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        RustExtractor::extract_with_edges(tree, content, file_path)
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[
            EdgeType::Imports,
            EdgeType::Calls,
            EdgeType::Implements,
            EdgeType::Uses,
        ]
    }

    fn language() -> Language {
        Language::Rust
    }
}

struct Collector<'a> {
    content: &'a str,
    file_path: &'a str,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    /// Callers in scope, innermost last: a function, or `None` for an impl/trait body, whose
    /// calls outside a function belong to the file (CRG `enclosing_func`).
    function_stack: Vec<Option<NodeId>>,
    references: bool,
    /// CRG `defined_names`: struct/enum/trait/fn names declared at the top of the file.
    defined_names: HashSet<String>,
    /// `m::item` for each struct/enum/trait/fn directly inside a top-level inline `mod m { }`.
    inline_module_items: HashSet<String>,
    /// `impl Trait for Type` blocks, as (type name, trait path segments), turned into
    /// `IMPLEMENTS` edges once every type node of the file exists.
    pending_implements: Vec<(String, Vec<String>, Span)>,
}

impl<'a> Collector<'a> {
    fn new(content: &'a str, file_path: &'a str) -> Self {
        Self {
            content,
            file_path,
            nodes: Vec::new(),
            edges: Vec::new(),
            function_stack: Vec::new(),
            references: true,
            defined_names: HashSet::new(),
            inline_module_items: HashSet::new(),
            pending_implements: Vec::new(),
        }
    }

    fn span_for(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
    }

    /// REVOLUTIONARY: Return both nodes and edges from single AST traversal
    fn into_result(mut self) -> ExtractionResult {
        // Like code-review-graph, `impl Trait for Type` links the type to the trait: the
        // file's own trait when it is a top-level name here, otherwise the path as written.
        for (type_name, trait_segments, span) in std::mem::take(&mut self.pending_implements) {
            let Some(from) = self
                .nodes
                .iter()
                .find(|n| {
                    n.name.as_str() == type_name
                        && matches!(
                            n.node_type,
                            Some(NodeType::Struct | NodeType::Class | NodeType::Trait)
                        )
                })
                .map(|n| n.id)
            else {
                continue;
            };
            let mut metadata = HashMap::from([
                ("source_file".to_string(), self.file_path.to_string()),
                ("implements_type".to_string(), "trait_impl".to_string()),
            ]);
            let to = match trait_segments.as_slice() {
                [name] if self.defined_names.contains(name) => {
                    format!("{}::{name}", self.file_path)
                }
                _ => {
                    metadata.insert(crg_shape::VERBATIM_TARGET_ATTR.into(), "true".into());
                    trait_segments.join("::")
                }
            };
            self.edges.push(EdgeRelationship {
                from,
                to,
                edge_type: EdgeType::Implements,
                metadata,
                span: Some(span),
            });
        }
        ExtractionResult {
            nodes: self.nodes,
            edges: self.edges,
        }
    }

    fn walk(&mut self, cursor: &mut TreeCursor, mut ctx: WalkContext) {
        let node = cursor.node();

        match node.kind() {
            // Modules
            // Like code-review-graph, a module is not a node; its items keep the file scope.
            "mod_item" => {
                if let Some(name) = self.child_text_by_kinds(node, &["identifier"]) {
                    // push module path for nested items only if there's a body
                    let has_body = node.child_by_field_name("body").is_some();
                    if has_body {
                        ctx.module_path.push(name);
                    }

                    if cursor.goto_first_child() {
                        loop {
                            self.walk(cursor, ctx.clone());
                            if !cursor.goto_next_sibling() {
                                break;
                            }
                        }
                        cursor.goto_parent();
                    }

                    if has_body {
                        ctx.module_path.pop();
                    }
                    return; // already recursed children
                }
            }

            // Imports (use)
            "use_declaration" => {
                let imports: Vec<String> = flatten_use(node, self.content, &[])
                    .into_iter()
                    .map(|(_, path)| path)
                    .collect();
                let Some(name) = imports.first().cloned() else {
                    return;
                };
                let loc = self.location(&node);
                let mut code =
                    CodeNode::new(name, Some(NodeType::Import), Some(Language::Rust), loc)
                        .with_content(self.node_text(&node));
                code.span = Some(self.span_for(&node));
                // Store full parsed import list and graph-friendly edges as metadata
                code.metadata
                    .attributes
                    .insert("imports".into(), json!(imports).to_string());
                code.metadata.attributes.insert(
                    "qualified_name".into(),
                    self.qname(&ctx.module_path, code.name.as_str()),
                );

                // REVOLUTIONARY: Extract import edges during same AST traversal
                let import_node_id = code.id;
                let import_span = self.span_for(&node);
                for import in &imports {
                    let edge = EdgeRelationship {
                        from: import_node_id,
                        to: import.clone(),
                        edge_type: EdgeType::Imports,
                        metadata: {
                            let mut meta = HashMap::new();
                            meta.insert("import_type".to_string(), "use_declaration".to_string());
                            meta.insert("source_file".to_string(), self.file_path.to_string());
                            meta
                        },
                        span: Some(import_span.clone()),
                    };
                    self.edges.push(edge);
                }

                self.nodes.push(code);
            }

            // Trait
            "trait_item" => {
                if let Some(name) = self.child_text_by_kinds(node, &["type_identifier"]) {
                    let loc = self.location(&node);
                    let mut code = CodeNode::new(
                        name.clone(),
                        Some(NodeType::Trait),
                        Some(Language::Rust),
                        loc,
                    )
                    .with_content(self.node_text(&node));
                    code.span = Some(self.span_for(&node));
                    let (generics, lifetimes) = self.collect_generics_and_lifetimes(node);
                    code.metadata
                        .attributes
                        .insert("generics".into(), json!(generics).to_string());
                    code.metadata
                        .attributes
                        .insert("lifetimes".into(), json!(lifetimes).to_string());
                    code.metadata
                        .attributes
                        .insert("qualified_name".into(), self.qname(&ctx.module_path, &name));
                    self.nodes.push(code);

                    let mut next_ctx = ctx.clone();
                    next_ctx.current_trait = Some(name.clone());
                    next_ctx.current_impl_for = None;
                    next_ctx.current_impl_trait = None;
                    next_ctx.class_name = Some(name);
                    self.function_stack.push(None);
                    self.walk_children(cursor, &next_ctx);
                    self.function_stack.pop();
                    return;
                }
            }

            // Struct
            "struct_item" => {
                if let Some(name) = self.child_text_by_kinds(node, &["type_identifier"]) {
                    let loc = self.location(&node);
                    let mut code = CodeNode::new(
                        name.clone(),
                        Some(NodeType::Struct),
                        Some(Language::Rust),
                        loc,
                    )
                    .with_content(self.node_text(&node));
                    code.span = Some(self.span_for(&node));
                    let (generics, lifetimes) = self.collect_generics_and_lifetimes(node);
                    code.metadata
                        .attributes
                        .insert("generics".into(), json!(generics).to_string());
                    code.metadata
                        .attributes
                        .insert("lifetimes".into(), json!(lifetimes).to_string());
                    code.metadata
                        .attributes
                        .insert("qualified_name".into(), self.qname(&ctx.module_path, &name));

                    // REVOLUTIONARY: Extract references to types in struct fields
                    let struct_node_id = code.id;
                    self.extract_references_from_fields(node, struct_node_id);

                    self.nodes.push(code);
                }
            }

            // Enum
            "enum_item" => {
                if let Some(name) = self.child_text_by_kinds(node, &["type_identifier"]) {
                    let loc = self.location(&node);
                    // code-review-graph stores a Rust enum as a Class.
                    let mut code = CodeNode::new(
                        name.clone(),
                        Some(NodeType::Class),
                        Some(Language::Rust),
                        loc,
                    )
                    .with_content(self.node_text(&node));
                    code.span = Some(self.span_for(&node));
                    let (generics, lifetimes) = self.collect_generics_and_lifetimes(node);
                    code.metadata
                        .attributes
                        .insert("generics".into(), json!(generics).to_string());
                    code.metadata
                        .attributes
                        .insert("lifetimes".into(), json!(lifetimes).to_string());
                    code.metadata
                        .attributes
                        .insert("qualified_name".into(), self.qname(&ctx.module_path, &name));
                    self.nodes.push(code);
                }
            }

            // Like code-review-graph, an impl is not a node: it only scopes its methods to the
            // type it names, wherever it is written.
            "impl_item" => {
                let impl_info = parse_impl_signature(&node, self.content)
                    .unwrap_or_else(|| parse_impl_signature_text(&self.node_text(&node)));
                let type_name = node
                    .child_by_field_name("type")
                    .map(|t| rust_path_segments(t, self.content))
                    .and_then(|segments| segments.last().cloned());
                if let (Some(type_name), Some(trait_node)) =
                    (&type_name, node.child_by_field_name("trait"))
                {
                    let trait_segments = rust_path_segments(trait_node, self.content);
                    if !trait_segments.is_empty() {
                        self.pending_implements.push((
                            type_name.clone(),
                            trait_segments,
                            self.span_for(&node),
                        ));
                    }
                }

                let mut next_ctx = ctx.clone();
                next_ctx.current_impl_for = impl_info.for_type;
                next_ctx.current_impl_trait = impl_info.trait_name;
                next_ctx.current_trait = None;
                next_ctx.class_name = type_name;
                self.function_stack.push(None);
                self.walk_children(cursor, &next_ctx);
                self.function_stack.pop();
                return;
            }

            // Functions (includes methods inside impl)
            "function_item" | "function_signature_item" => {
                if let Some(name) = self.child_text_by_kinds(node, &["identifier"]) {
                    let loc = self.location(&node);
                    let mut code = CodeNode::new(
                        name.clone(),
                        Some(NodeType::Function),
                        Some(Language::Rust),
                        loc,
                    )
                    .with_content(self.node_text(&node));
                    if node.kind() == "function_item" {
                        code = code.with_complexity(
                            crate::complexity::calculate_cyclomatic_complexity(&node, self.content),
                        );
                    }
                    let decorators = rust_attributes(node, self.content);
                    if !decorators.is_empty() {
                        code.metadata
                            .attributes
                            .insert(crg_shape::DECORATORS_ATTR.into(), decorators.join(","));
                    }
                    let function_span = self.span_for(&node);
                    code.span = Some(function_span.clone());
                    let (generics, lifetimes) = self.collect_generics_and_lifetimes(node);
                    code.metadata
                        .attributes
                        .insert("generics".into(), json!(generics).to_string());
                    code.metadata
                        .attributes
                        .insert("lifetimes".into(), json!(lifetimes).to_string());

                    // REVOLUTIONARY: Extract references from parameters and return type
                    self.extract_references_from_signature(node, code.id);

                    let is_async =
                        self.node_text(&node).contains("async fn") || has_child_kind(node, "async");
                    let is_unsafe = self.node_text(&node).contains("unsafe fn")
                        || has_child_kind(node, "unsafe");
                    let has_unsafe_block = subtree_contains_kind(&node, "unsafe_block");
                    code.metadata
                        .attributes
                        .insert("async".into(), json!(is_async).to_string());
                    code.metadata
                        .attributes
                        .insert("unsafe".into(), json!(is_unsafe).to_string());
                    code.metadata.attributes.insert(
                        "has_unsafe_block".into(),
                        json!(has_unsafe_block).to_string(),
                    );
                    if let Some(owner) =
                        ctx.current_impl_for.as_ref().or(ctx.current_trait.as_ref())
                    {
                        code.metadata
                            .attributes
                            .insert("method_of".into(), owner.clone());
                    }
                    if let Some(trait_name) = &ctx.current_impl_trait {
                        code.metadata
                            .attributes
                            .insert("implements_trait".into(), trait_name.clone());
                    }
                    code.metadata
                        .attributes
                        .insert("qualified_name".into(), self.qname_with_impl(&ctx, &name));

                    let function_node_id = code.id;
                    self.nodes.push(code);

                    self.function_stack.push(Some(function_node_id));
                    self.walk_children(cursor, &ctx);
                    self.function_stack.pop();
                    return;
                }
            }

            "call_expression" | "macro_invocation" => self.emit_call(node, &ctx),

            _ => {}
        }

        // Recurse into children generically
        if cursor.goto_first_child() {
            loop {
                self.walk(cursor, ctx.clone());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
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

    fn child_text_by_kinds(&self, node: Node, kinds: &[&str]) -> Option<String> {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let n = c.node();
                if kinds.iter().any(|k| n.kind() == *k) {
                    return Some(self.node_text(&n));
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    fn collect_generics_and_lifetimes(&self, node: Node) -> (Vec<String>, Vec<String>) {
        let mut generics = Vec::new();
        let mut lifetimes = Vec::new();

        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                let n = cursor.node();
                match n.kind() {
                    "type_parameters" => {
                        self.collect_type_parameters(n, &mut generics, &mut lifetimes);
                    }
                    _ => {}
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        (generics, lifetimes)
    }

    fn collect_type_parameters(
        &self,
        node: Node,
        generics: &mut Vec<String>,
        lifetimes: &mut Vec<String>,
    ) {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let n = c.node();
                match n.kind() {
                    "type_parameter" => {
                        if let Some(t) = self.child_text_by_kinds(n, &["type_identifier"]) {
                            generics.push(t);
                        }
                    }
                    "lifetime_parameter" => {
                        if let Some(lf) = self.child_text_by_kinds(n, &["lifetime"]) {
                            lifetimes.push(lf);
                        }
                    }
                    "const_parameter" => {
                        if let Some(id) = self.child_text_by_kinds(n, &["identifier"]) {
                            generics.push(format!("const {}", id));
                        }
                    }
                    _ => {}
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn qname(&self, path: &[String], name: &str) -> String {
        if path.is_empty() {
            format!("{}::{}", self.file_path, name)
        } else {
            format!("{}::{}::{}", self.file_path, path.join("::"), name)
        }
    }

    fn qname_with_impl(&self, ctx: &WalkContext, name: &str) -> String {
        match (
            &ctx.current_impl_for,
            &ctx.current_impl_trait,
            &ctx.current_trait,
        ) {
            (Some(t), Some(tr), _) => {
                format!("{}::impl<{} for {}>::{}", self.file_path, tr, t, name)
            }
            (Some(t), None, _) => format!("{}::impl<{}>::{}", self.file_path, t, name),
            (None, _, Some(tr)) => format!("{}::trait<{}>::{}", self.file_path, tr, name),
            _ => self.qname(&ctx.module_path, name),
        }
    }

    fn walk_children(&mut self, cursor: &mut TreeCursor, ctx: &WalkContext) {
        if cursor.goto_first_child() {
            loop {
                self.walk(cursor, ctx.clone());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
    }

    /// A CALLS edge as code-review-graph records it: path name (`HashMap::new`), method name
    /// with its `receiver`, or macro name; `self`/`Self::` and paths through the file's own
    /// types or inline modules resolved here, other paths kept as verbatim text.
    fn emit_call(&mut self, node: Node, ctx: &WalkContext) {
        if is_in_rust_dead_guard(node, self.content) {
            return;
        }
        let (name, receiver) = if node.kind() == "macro_invocation" {
            let Some(path) = node.child_by_field_name("macro") else {
                return;
            };
            let segments = rust_path_segments(path, self.content);
            if segments.is_empty() {
                (self.node_text(&path), None)
            } else {
                (segments.join("::"), None)
            }
        } else {
            let Some(mut callee) = node.child_by_field_name("function") else {
                return;
            };
            let segments = rust_path_segments(callee, self.content);
            if !segments.is_empty() {
                (segments.join("::"), None)
            } else {
                while callee.kind() == "generic_function" {
                    match callee.child_by_field_name("function") {
                        Some(inner) => callee = inner,
                        None => return,
                    }
                }
                let (Some(value), Some(field)) = (
                    callee.child_by_field_name("value"),
                    callee.child_by_field_name("field"),
                ) else {
                    return;
                };
                if callee.kind() != "field_expression" {
                    return;
                }
                (self.node_text(&field), Some(self.node_text(&value)))
            }
        };

        let mut verbatim = false;
        let target = match (&receiver, &ctx.class_name) {
            (Some(r), Some(class)) if r == "self" => format!("{}::{class}.{name}", self.file_path),
            (None, Some(class)) if name.starts_with("Self::") => {
                let method = name.rsplit("::").next().unwrap_or(&name);
                format!("{}::{class}.{method}", self.file_path)
            }
            (Some(_), _) => name.clone(),
            (None, _) if name.contains("::") => {
                self.resolve_scoped_call(&name).unwrap_or_else(|| {
                    verbatim = true;
                    name.clone()
                })
            }
            (None, _) => name.clone(),
        };

        let mut metadata = HashMap::from([
            (
                "call_type".to_string(),
                if node.kind() == "macro_invocation" {
                    "macro"
                } else {
                    "function_call"
                }
                .to_string(),
            ),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        if let Some(receiver) = receiver {
            if receiver != "self" && receiver != "super" {
                metadata.insert(crg_shape::UNTYPED_RECEIVER_ATTR.into(), "true".into());
            }
            metadata.insert("receiver".into(), receiver);
        }
        if verbatim {
            metadata.insert(crg_shape::VERBATIM_TARGET_ATTR.into(), "true".into());
        }
        let from = match self.function_stack.last() {
            Some(Some(id)) => *id,
            _ => {
                metadata.insert(crg_shape::FILE_SCOPE_ATTR.into(), "true".into());
                NodeId::nil()
            }
        };
        self.edges.push(EdgeRelationship {
            from,
            to: target,
            edge_type: EdgeType::Calls,
            metadata,
            span: Some(self.span_for(&node)), // Attach span for LSP resolution
        });
    }

    /// CRG `_resolve_rust_scoped_call`, within this file: `Type::m` for a top-level type or
    /// function of the file, `m::item` for an item of an inline module.
    fn resolve_scoped_call(&self, name: &str) -> Option<String> {
        let parts: Vec<&str> = name.split("::").filter(|p| !p.is_empty()).collect();
        let (method, prefix) = parts.split_last()?;
        match prefix {
            [] | ["Self"] => None,
            [owner] if self.defined_names.contains(*owner) => {
                Some(format!("{}::{owner}.{method}", self.file_path))
            }
            [module]
                if self
                    .inline_module_items
                    .contains(&format!("{module}::{method}")) =>
            {
                Some(format!("{}::{method}", self.file_path))
            }
            _ => None,
        }
    }

    /// REVOLUTIONARY: Extract references to types in function parameters and return type
    fn extract_references_from_signature(&mut self, node: Node, from_id: NodeId) {
        // Parameters
        if let Some(params) = node.child_by_field_name("parameters") {
            let mut cursor = params.walk();
            if cursor.goto_first_child() {
                loop {
                    let param = cursor.node();
                    if param.kind() == "parameter" {
                        if let Some(type_node) = param.child_by_field_name("type") {
                            self.extract_type_references(type_node, from_id, "parameter_type");
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        // Return type
        if let Some(ret) = node.child_by_field_name("return_type") {
            self.extract_type_references(ret, from_id, "return_type");
        }
    }

    /// REVOLUTIONARY: Extract references to types in struct fields
    fn extract_references_from_fields(&mut self, node: Node, from_id: NodeId) {
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            if cursor.goto_first_child() {
                loop {
                    let field = cursor.node();
                    if field.kind() == "field_declaration" {
                        if let Some(type_node) = field.child_by_field_name("type") {
                            self.extract_type_references(type_node, from_id, "field_type");
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }

    /// Deep scan for type identifiers within a type node (handles generics, pointers, etc.)
    fn extract_type_references(&mut self, node: Node, from_id: NodeId, kind: &str) {
        if !self.references {
            return;
        }
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            match n.kind() {
                "type_identifier" | "primitive_type" => {
                    let name = self.node_text(&n);
                    if !name.is_empty() {
                        self.edges.push(EdgeRelationship {
                            from: from_id,
                            to: name,
                            edge_type: EdgeType::References,
                            metadata: HashMap::from([
                                ("kind".into(), kind.into()),
                                ("source_file".into(), self.file_path.into()),
                            ]),
                            span: Some(self.span_for(&n)),
                        });
                    }
                }
                _ => {
                    let mut cursor = n.walk();
                    if cursor.goto_first_child() {
                        loop {
                            stack.push(cursor.node());
                            if !cursor.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// CRG `_rust_path_segments`: the semantic segments of a Rust path, without type arguments.
fn rust_path_segments(node: Node, content: &str) -> Vec<String> {
    let text = |n: Node| n.utf8_text(content.as_bytes()).unwrap_or("").to_string();
    match node.kind() {
        "identifier" | "type_identifier" | "crate" | "self" | "super" => vec![text(node)],
        "scoped_identifier" | "scoped_type_identifier" => {
            let mut segments = node
                .child_by_field_name("path")
                .map(|p| rust_path_segments(p, content))
                .unwrap_or_default();
            if let Some(name) = node.child_by_field_name("name") {
                segments.extend(rust_path_segments(name, content));
            }
            segments
        }
        "generic_type" => node
            .child_by_field_name("type")
            .map(|t| rust_path_segments(t, content))
            .unwrap_or_default(),
        "generic_function" => node
            .child_by_field_name("function")
            .map(|f| rust_path_segments(f, content))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// CRG decorators of a Rust item: the `#[...]` attributes right before it, nearest first,
/// without the `#[`/`#![` and `]` wrapper.
fn rust_attributes(node: Node, content: &str) -> Vec<String> {
    let mut attributes = Vec::new();
    let mut sibling = node.prev_sibling();
    while let Some(attribute) = sibling.filter(|s| s.kind() == "attribute_item") {
        let text = attribute.utf8_text(content.as_bytes()).unwrap_or("").trim();
        let inner = text
            .strip_prefix("#![")
            .or_else(|| text.strip_prefix("#["))
            .map(str::trim_end)
            .unwrap_or(text);
        attributes.push(inner.strip_suffix(']').unwrap_or(inner).trim().to_string());
        sibling = attribute.prev_sibling();
    }
    attributes
}

/// CRG `defined_names` for Rust: struct/enum/trait/fn names that are direct children of the
/// file root.
fn defined_names(root: Node, content: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut cursor = root.walk();
    for item in root.named_children(&mut cursor) {
        if !matches!(
            item.kind(),
            "struct_item"
                | "enum_item"
                | "trait_item"
                | "function_item"
                | "function_signature_item"
        ) {
            continue;
        }
        if let Some(name) = item.child_by_field_name("name") {
            names.insert(name.utf8_text(content.as_bytes()).unwrap_or("").to_string());
        }
    }
    names
}

/// CRG `_RUST_INLINE_MODULE_KEY` entries: `m::item` for each struct/enum/trait/fn directly in
/// the body of an inline `mod m { }` at the file root.
fn inline_module_items(root: Node, content: &str) -> HashSet<String> {
    let text = |n: Node| n.utf8_text(content.as_bytes()).unwrap_or("").to_string();
    let mut items = HashSet::new();
    let mut cursor = root.walk();
    for module in root.named_children(&mut cursor) {
        let (Some(name), Some(body)) = (
            module.child_by_field_name("name"),
            module.child_by_field_name("body"),
        ) else {
            continue;
        };
        if module.kind() != "mod_item" {
            continue;
        }
        let mut body_cursor = body.walk();
        for item in body.named_children(&mut body_cursor) {
            if !matches!(
                item.kind(),
                "struct_item"
                    | "enum_item"
                    | "trait_item"
                    | "function_item"
                    | "function_signature_item"
            ) {
                continue;
            }
            if let Some(item_name) = item.child_by_field_name("name") {
                items.insert(format!("{}::{}", text(name), text(item_name)));
            }
        }
    }
    items
}

/// CRG `_is_in_rust_dead_guard`: inside the consequence of an `if false` (parentheses
/// unwrapped). The walk stops at an `impl_item`: an impl written in a dead branch still
/// applies to its type, so its methods stay live.
fn is_in_rust_dead_guard(node: Node, content: &str) -> bool {
    let mut cursor = node.parent();
    while let Some(current) = cursor {
        if current.kind() == "impl_item" {
            return false;
        }
        if current.kind() == "if_expression" {
            let mut condition = current.child_by_field_name("condition");
            while let Some(paren) = condition.filter(|c| c.kind() == "parenthesized_expression") {
                let mut walk = paren.walk();
                condition = paren.named_children(&mut walk).next();
            }
            if let (Some(condition), Some(consequence)) =
                (condition, current.child_by_field_name("consequence"))
                && condition.kind() == "boolean_literal"
                && condition.utf8_text(content.as_bytes()) == Ok("false")
                && consequence.start_byte() <= node.start_byte()
                && node.end_byte() <= consequence.end_byte()
            {
                return true;
            }
        }
        cursor = current.parent();
    }
    false
}

/// CRG `_parse_rust_use_node`: a `use` tree flattened into `(local name, original path)`
/// pairs. A glob has no `path` field, so a top-level `use a::*` yields nothing and a nested one
/// (`use a::{b::*}`) yields the group prefix.
fn flatten_use(node: Node, content: &str, prefix: &[String]) -> Vec<(String, String)> {
    let joined = |extra: &[String]| [prefix, extra].concat().join("::");
    match node.kind() {
        "use_declaration" => node
            .child_by_field_name("argument")
            .map(|argument| flatten_use(argument, content, prefix))
            .unwrap_or_default(),
        "identifier" | "type_identifier" => {
            let name = node.utf8_text(content.as_bytes()).unwrap_or("").to_string();
            vec![(name.clone(), joined(&[name]))]
        }
        "self" => match prefix.last() {
            Some(last) => vec![(last.clone(), joined(&[]))],
            None => Vec::new(),
        },
        "scoped_identifier" | "scoped_type_identifier" => {
            let segments = rust_path_segments(node, content);
            match segments.last() {
                Some(last) => vec![(last.clone(), joined(&segments))],
                None => Vec::new(),
            }
        }
        "use_as_clause" => {
            let segments = node
                .child_by_field_name("path")
                .map(|path| rust_path_segments(path, content))
                .unwrap_or_default();
            match node.child_by_field_name("alias") {
                Some(alias) if !segments.is_empty() => vec![(
                    alias
                        .utf8_text(content.as_bytes())
                        .unwrap_or("")
                        .to_string(),
                    joined(&segments),
                )],
                _ => Vec::new(),
            }
        }
        "scoped_use_list" => {
            let mut nested = prefix.to_vec();
            if let Some(path) = node.child_by_field_name("path") {
                nested.extend(rust_path_segments(path, content));
            }
            node.child_by_field_name("list")
                .map(|list| flatten_use(list, content, &nested))
                .unwrap_or_default()
        }
        "use_list" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .flat_map(|child| flatten_use(child, content, prefix))
                .collect()
        }
        "use_wildcard" if !prefix.is_empty() => vec![("*".into(), joined(&[]))],
        _ => Vec::new(),
    }
}

#[derive(Default, Debug)]
struct ImplInfo {
    trait_name: Option<String>,
    for_type: Option<String>,
    generics: Vec<String>,
    lifetimes: Vec<String>,
}

fn parse_impl_signature(node: &Node, content: &str) -> Option<ImplInfo> {
    // Prefer AST-based extraction using fields when available
    let mut info = ImplInfo::default();
    if let Some(tp) = node.child_by_field_name("type_parameters") {
        // Collect generics and lifetimes
        let mut c = tp.walk();
        if c.goto_first_child() {
            loop {
                let n = c.node();
                match n.kind() {
                    "type_parameter" => {
                        if let Some(t) = child_text_by_kinds(n, content, &["type_identifier"]) {
                            info.generics.push(t);
                        }
                    }
                    "lifetime_parameter" => {
                        if let Some(lf) = child_text_by_kinds(n, content, &["lifetime"]) {
                            info.lifetimes.push(lf);
                        }
                    }
                    _ => {}
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    if let Some(tr) = node.child_by_field_name("trait") {
        info.trait_name = Some(tr.utf8_text(content.as_bytes()).ok()?.to_string());
    }
    if let Some(ty) = node.child_by_field_name("type") {
        info.for_type = Some(ty.utf8_text(content.as_bytes()).ok()?.to_string());
    }
    Some(info)
}

fn parse_impl_signature_text(text: &str) -> ImplInfo {
    // Fallback robust text-based parse
    // Matches forms like: impl<T> Trait for Type { ... } and impl<T> Type { ... }
    let mut info = ImplInfo::default();
    // generics
    if let Some(start) = text.find('<') {
        if let Some(end) = matching_angle(text, start) {
            let inside = &text[start + 1..end];
            for tok in inside.split(',') {
                let t = tok.trim();
                if t.is_empty() {
                    continue;
                }
                if t.starts_with('\'') {
                    info.lifetimes.push(t.to_string());
                } else if t.starts_with("const ") {
                    info.generics.push(t.to_string());
                } else {
                    // remove bounds (after ':') for a clean name
                    let name = t.split(':').next().unwrap_or(t).trim().to_string();
                    info.generics.push(name);
                }
            }
        }
    }
    let s = text.replace('\n', " ");
    if let Some(idx) = s.find(" for ") {
        // trait impl
        let head = s.trim_start_matches("impl").trim();
        let trait_part = head[..idx - 0].trim();
        let after_for = &s[idx + 5..];
        let ty = after_for.split('{').next().unwrap_or(after_for).trim();
        info.trait_name = Some(trait_part.to_string());
        info.for_type = Some(ty.trim_end().to_string());
    } else {
        // inherent impl: impl Type { ... }
        if let Some(after_impl) = s.strip_prefix("impl ") {
            let ty = after_impl.split('{').next().unwrap_or(after_impl).trim();
            info.for_type = Some(ty.to_string());
        }
    }
    info
}

fn matching_angle(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, ch) in text.chars().enumerate().skip(open) {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn has_child_kind(node: Node, kind: &str) -> bool {
    let mut c = node.walk();
    if c.goto_first_child() {
        loop {
            if c.node().kind() == kind {
                return true;
            }
            if !c.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

fn subtree_contains_kind(node: &Node, kind: &str) -> bool {
    // DFS
    let mut stack = vec![*node];
    while let Some(n) = stack.pop() {
        if n.kind() == kind {
            return true;
        }
        let mut w = n.walk();
        if w.goto_first_child() {
            loop {
                stack.push(w.node());
                if !w.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    false
}

fn child_text_by_kinds(node: Node, content: &str, kinds: &[&str]) -> Option<String> {
    let mut c = node.walk();
    if c.goto_first_child() {
        loop {
            let n = c.node();
            if kinds.iter().any(|k| n.kind() == *k) {
                return n.utf8_text(content.as_bytes()).ok().map(|s| s.to_string());
            }
            if !c.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use codegraph_core::{EdgeType, ExtractionResult, Language, NodeType};
    use std::collections::BTreeSet;

    fn extract_rust(source: &str, path: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        crate::languages::extract_for_language_at(
            &Language::Rust,
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
    fn impl_mod_and_macros_are_not_nodes_and_methods_hang_off_the_type() {
        let r = extract_rust(
            "pub trait Repo { fn find(&self); fn save(&mut self) {} }\n\
             pub struct Mem;\nimpl Mem { pub fn new() -> Self { Mem } }\n\
             impl Repo for Mem { fn find(&self) { println!(\"x\"); } }\n\
             #[cfg(test)]\nmod tests { #[test] fn t() { assert!(true); } }\n",
            "m.rs",
        );
        let names: BTreeSet<_> = r
            .nodes
            .iter()
            .filter_map(|n| n.metadata.attributes.get("qualified_name").cloned())
            .collect();
        assert_eq!(
            names,
            set(&[
                "m.rs",
                "m.rs::Repo",
                "m.rs::Repo.find",
                "m.rs::Repo.save",
                "m.rs::Mem",
                "m.rs::Mem.new",
                "m.rs::Mem.find",
                "m.rs::t",
            ])
        );
        let contains = edges(&r, EdgeType::Contains);
        assert!(contains.contains(&("m.rs::Mem".into(), "m.rs::Mem.find".into())));
        assert!(contains.contains(&("m.rs::Repo".into(), "m.rs::Repo.find".into())));
        assert_eq!(
            edges(&r, EdgeType::Implements),
            BTreeSet::from([("m.rs::Mem".into(), "m.rs::Repo".into())])
        );
    }

    #[test]
    fn test_attributes_make_tests() {
        let r = extract_rust(
            "#[test]\nfn a() {}\n#[tokio::test]\nasync fn b() {}\n#[test]\n// gap\nfn c() {}\n\
             #[inline]\nfn d() {}\n",
            "src/m.rs",
        );
        let tests: BTreeSet<_> = r
            .nodes
            .iter()
            .filter(|n| n.node_type == Some(NodeType::Other("Test".into())))
            .map(|n| n.name.to_string())
            .collect();
        assert_eq!(tests, set(&["a", "b"]));
    }

    #[test]
    fn generic_and_path_impls_use_the_last_segment() {
        let r = extract_rust(
            "struct Foo<T>(T);\nimpl<T> Foo<T> { fn get(&self) {} }\n\
             impl<T> fmt::Display for Foo<T> { fn fmt(&self) {} }\n",
            "m.rs",
        );
        assert!(
            edges(&r, EdgeType::Contains).contains(&("m.rs::Foo".into(), "m.rs::Foo.get".into()))
        );
        assert_eq!(
            edges(&r, EdgeType::Implements),
            BTreeSet::from([("m.rs::Foo".into(), "fmt::Display".into())])
        );
    }

    #[test]
    fn call_names_callers_and_targets_follow_crg() {
        let r = extract_rust(
            "mod util { pub fn h() {} }\nstruct T;\n\
             impl T { fn m(&self) { self.m(); Self::m(self); self.v.get(); } const C: u8 = mk(); }\n\
             fn mk() -> u8 { util::h(); tokio::select!{}; println!(\"x\"); crate::x::y();\n\
               Vec::<u8>::new(); g::<u8>(); T::m(&T); HashMap::new(); (f)(); 1 }\nfn g<X>() {}\n",
            "m.rs",
        );
        assert_eq!(calls_from(&r, "m.rs::T.m"), set(&["m.rs::T.m", "get"]));
        assert_eq!(calls_from(&r, "m.rs"), set(&["mk"]));
        assert_eq!(
            calls_from(&r, "m.rs::mk"),
            set(&[
                "m.rs::h",
                "tokio::select",
                "println",
                "crate::x::y",
                "Vec::new",
                "g",
                "m.rs::T.m",
                "HashMap::new",
            ])
        );
        let verbatim: BTreeSet<_> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .filter(|e| {
                e.metadata
                    .contains_key(crate::languages::crg_shape::VERBATIM_TARGET_ATTR)
            })
            .map(|e| e.to.clone())
            .collect();
        assert_eq!(
            verbatim,
            set(&["tokio::select", "crate::x::y", "Vec::new", "HashMap::new"])
        );
        let get = r.edges.iter().find(|e| e.to == "get").unwrap();
        assert_eq!(
            get.metadata.get("receiver").map(String::as_str),
            Some("self.v")
        );
    }

    #[test]
    fn call_after_nested_impl_leaves_the_outer_fn() {
        let r = extract_rust(
            "struct S;\nfn live() {}\nfn main() {\n if false { impl S { fn run() { live(); } } }\n S::run();\n}\n",
            "m.rs",
        );
        assert_eq!(calls_from(&r, "m.rs::main"), set(&["m.rs::S.run"]));
        assert_eq!(calls_from(&r, "m.rs::S.run"), set(&["live"]));
    }

    #[test]
    fn calls_under_if_false_are_dead() {
        let r = extract_rust(
            "fn a(){} fn b(){} fn c(){} fn d(){}\n\
             fn f() { if false { a(); x!(); } else { b(); } if true { c(); } if false && false { d(); } }\n",
            "m.rs",
        );
        assert_eq!(calls_from(&r, "m.rs::f"), set(&["b", "c", "d"]));
    }

    #[test]
    fn use_trees_flatten_like_crg() {
        let r = extract_rust(
            "use std::collections::HashMap;\nuse a::{b, c::D, self as e};\nuse a::{self};\n\
             use x::Y as Z;\nuse p::{q::{r, s}};\nuse super::*;\nuse std::io::*;\nuse k::{l::*};\n",
            "m.rs",
        );
        let imports = edges(&r, EdgeType::Imports);
        let targets: BTreeSet<_> = imports.iter().map(|(_, to)| to.clone()).collect();
        assert_eq!(
            targets,
            set(&[
                "std::collections::HashMap",
                "a::b",
                "a::c::D",
                "a::self",
                "a",
                "x::Y",
                "p::q::r",
                "p::q::s",
                "k",
            ])
        );
        assert!(imports.iter().all(|(from, _)| from == "m.rs"));
    }

    #[test]
    fn enums_are_classes() {
        let r = extract_rust("pub enum E { X }\nimpl E { fn run() {} }\n", "m.rs");
        let e = r.nodes.iter().find(|n| n.name.as_str() == "E").unwrap();
        assert_eq!(e.node_type, Some(NodeType::Class));
        assert!(edges(&r, EdgeType::Contains).contains(&("m.rs::E".into(), "m.rs::E.run".into())));
    }

    #[test]
    fn calls_on_other_receivers_are_marked_untyped() {
        let r = extract_rust(
            "struct T;\nimpl T { fn m(&self) { self.m(); self.v.get(); } }\nfn f(x: T) { x.m(); }\n",
            "m.rs",
        );
        let untyped: BTreeSet<_> = r
            .edges
            .iter()
            .filter(|e| {
                e.metadata
                    .contains_key(crate::languages::crg_shape::UNTYPED_RECEIVER_ATTR)
            })
            .map(|e| (qualified(&r, &e.from), e.to.clone()))
            .collect();
        assert_eq!(
            untyped,
            BTreeSet::from([
                ("m.rs::T.m".to_string(), "get".to_string()),
                ("m.rs::f".to_string(), "m".to_string()),
            ])
        );
    }
}
