// ABOUTME: Java AST extractor shaped like code-review-graph's Java parser.
// ABOUTME: Classes, methods, imports, calls with typed receivers, INHERITS and Spring INJECTS.

use super::crg_shape;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Tree};

/// Java extractor following code-review-graph (CRG): no package nodes, calls named like
/// `_get_call_name`, receivers bound to declared types (`_collect_typed_call_targets`) and
/// receiver-less calls bound to the first same-named definition in the file
/// (`_resolve_call_targets`). Spring DI calls are resolved later, across files, by the indexer.
pub struct JavaExtractor;

/// Edge kinds CRG emits from Java framework annotations and fields.
pub const INJECTS: &str = "injects";
pub const CONSUMES: &str = "consumes";
pub const PRODUCES: &str = "produces";
pub const DEPENDS_ON_CONFIG: &str = "depends_on_config";
pub const HANDLES: &str = "handles";
pub const TRIGGERS: &str = "triggers";
pub const PUBLISHES: &str = "publishes";
pub const TEMPORAL_STUB: &str = "temporal_stub";

/// CRG keeps textual targets for every edge kind it extracts from Java source.
pub fn is_framework_edge(kind: &str) -> bool {
    matches!(
        kind,
        INJECTS
            | CONSUMES
            | PRODUCES
            | DEPENDS_ON_CONFIG
            | HANDLES
            | TRIGGERS
            | PUBLISHES
            | TEMPORAL_STUB
    )
}

const KAFKA_LISTENER_ANNOTATIONS: &[&str] = &["KafkaListener", "KafkaHandler"];
const KAFKA_CONSUMER_TYPES: &[&str] = &["KafkaReceiver", "ReactiveKafkaConsumerTemplate"];
const KAFKA_PRODUCER_TYPES: &[&str] = &[
    "KafkaTemplate",
    "KafkaOperations",
    "ReactiveKafkaProducerTemplate",
];
const REQUEST_MAPPINGS: &[&str] = &[
    "GetMapping",
    "PostMapping",
    "PutMapping",
    "DeleteMapping",
    "PatchMapping",
    "RequestMapping",
];

/// CRG `_SPRING_INJECT_ANNOTATIONS`.
const INJECT_ANNOTATIONS: &[&str] = &["Autowired", "Inject", "Resource"];

/// CRG `_TRANSPARENT_TYPE_WRAPPERS`.
const TRANSPARENT_WRAPPERS: &[&str] = &["Annotated", "Optional", "Type"];

/// CRG `_NON_RECEIVER_TYPE_NAMES`.
const NON_RECEIVER_TYPES: &[&str] = &[
    "Array",
    "Collection",
    "Dict",
    "Iterable",
    "List",
    "Map",
    "Mapping",
    "MutableList",
    "MutableMap",
    "MutableMapping",
    "ReadonlyArray",
    "Sequence",
    "Set",
    "Tuple",
    "Union",
    "dict",
    "frozenset",
    "list",
    "set",
    "tuple",
];

/// Scalar names `_base_type_name` never treats as a receiver class.
const SCALAR_TYPES: &[&str] = &[
    "None", "bool", "boolean", "float", "int", "number", "str", "string", "unknown", "void",
];

impl JavaExtractor {
    pub fn extract(tree: &Tree, content: &str, file_path: &str) -> Vec<CodeNode> {
        Self::extract_with_edges(tree, content, file_path).nodes
    }

    /// Unified extraction of nodes AND edges in single AST traversal
    pub fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        let root = tree.root_node();
        let mut collector = JavaCollector::new(content, file_path, &root);
        collector.visit(root, &mut Bindings::new(), &Bindings::new());
        collector.bind_bare_calls();
        ExtractionResult {
            nodes: collector.nodes,
            edges: collector.edges,
        }
    }
}

impl super::LanguageExtractor for JavaExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        JavaExtractor::extract_with_edges(tree, content, file_path)
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[EdgeType::Imports, EdgeType::Calls, EdgeType::Extends]
    }

    fn language() -> Language {
        Language::Java
    }
}

/// Variable name -> declared receiver type (a base class name, or a package-qualified one).
type Bindings = HashMap<String, String>;

struct JavaCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    package: Option<String>,
    /// CRG Java `import_map`: simple name -> imported name; static imports are left out.
    imports: HashMap<String, String>,
    wildcard_import: bool,
    /// Type names declared anywhere in this file.
    defined: HashSet<String>,
    /// Enclosing methods/constructors, innermost last.
    functions: Vec<NodeId>,
    /// Enclosing class names, outermost first.
    classes: Vec<String>,
    /// `(name, qualified name)` of every class and method, in file order.
    symbols: Vec<(String, String)>,
    /// Indices of CALLS edges written without a receiver.
    bare_calls: Vec<usize>,
    /// Virtual `event::` nodes already emitted in this file.
    virtual_events: HashMap<String, NodeId>,
}

impl<'a> JavaCollector<'a> {
    fn new(content: &'a str, file_path: &'a str, root: &Node) -> Self {
        let mut collector = Self {
            content,
            file_path,
            nodes: Vec::new(),
            edges: Vec::new(),
            package: None,
            imports: HashMap::new(),
            wildcard_import: false,
            defined: HashSet::new(),
            functions: Vec::new(),
            classes: Vec::new(),
            symbols: Vec::new(),
            bare_calls: Vec::new(),
            virtual_events: HashMap::new(),
        };
        collector.scan_declarations(root);
        collector
    }

    /// Package, imports and declared type names, known before any call is visited.
    fn scan_declarations(&mut self, root: &Node) {
        for child in named_children(root) {
            match child.kind() {
                "package_declaration" => {
                    self.package = named_children(&child)
                        .find(|c| matches!(c.kind(), "scoped_identifier" | "identifier"))
                        .map(|c| self.node_text(&c));
                }
                "import_declaration" => {
                    let text = self.node_text(&child);
                    let Some(imported) = text.trim().strip_prefix("import ") else {
                        continue;
                    };
                    let imported = imported.trim_end_matches(';').trim();
                    if imported.starts_with("static ") {
                        continue;
                    }
                    if imported.ends_with(".*") {
                        self.wildcard_import = true;
                        continue;
                    }
                    let local = imported.rsplit('.').next().unwrap_or(imported);
                    if !local.is_empty() {
                        self.imports.insert(local.to_string(), imported.to_string());
                    }
                }
                _ => {}
            }
        }
        let mut stack = vec![*root];
        while let Some(node) = stack.pop() {
            if is_class(node.kind())
                && let Some(name) = node.child_by_field_name("name")
            {
                self.defined.insert(self.node_text(&name));
            }
            stack.extend(named_children(&node));
        }
    }

    fn visit(&mut self, node: Node, bindings: &mut Bindings, class_fields: &Bindings) {
        match node.kind() {
            "package_declaration" => return,
            "import_declaration" => {
                self.import(&node);
                return;
            }
            kind if is_class(kind) => {
                let fields = self.class_typed_fields(&node);
                let mut class_bindings = bindings.clone();
                class_bindings.extend(fields.clone());
                let pushed = self.class(&node);
                for child in children(&node) {
                    self.visit(child, &mut class_bindings, &fields);
                }
                if pushed {
                    self.classes.pop();
                }
                return;
            }
            "method_declaration" | "constructor_declaration" => {
                let mut scoped = bindings.clone();
                scoped.extend(class_fields.clone());
                scoped.extend(self.typed_parameters(&node));
                let pushed = self.function(&node);
                for child in children(&node) {
                    self.visit(child, &mut scoped, class_fields);
                }
                if pushed {
                    self.functions.pop();
                }
                return;
            }
            "block" => {
                let mut scoped = bindings.clone();
                for child in children(&node) {
                    self.visit(child, &mut scoped, class_fields);
                }
                return;
            }
            "method_invocation" | "object_creation_expression" | "method_reference" => {
                self.call(&node, bindings, class_fields);
            }
            _ => {
                let declared = self.typed_bindings(&node);
                if !declared.is_empty() {
                    // Initializers are evaluated before their declaration becomes visible.
                    for child in children(&node) {
                        self.visit(child, bindings, class_fields);
                    }
                    bindings.extend(declared);
                    return;
                }
            }
        }
        for child in children(&node) {
            self.visit(child, bindings, class_fields);
        }
    }

    fn import(&mut self, node: &Node) {
        let content_text = self.node_text(node);
        let is_static = content_text.contains("static ");
        let is_wildcard = content_text.contains(".*");
        let Some(path_node) =
            named_children(node).find(|c| matches!(c.kind(), "scoped_identifier" | "identifier"))
        else {
            return;
        };
        let path = self.node_text(&path_node);
        let mut code = CodeNode::new(
            format!("import {path}"),
            Some(NodeType::Import),
            Some(Language::Java),
            self.location(node),
        )
        .with_content(content_text);
        code.span = Some(self.span_for(node));
        let attributes = &mut code.metadata.attributes;
        attributes.insert("kind".into(), "import".into());
        attributes.insert("path".into(), path.clone());
        let mut metadata = HashMap::from([
            ("import_type".to_string(), "java_import".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        if is_static {
            attributes.insert("static".into(), "true".into());
            metadata.insert("static".into(), "true".into());
        }
        if is_wildcard {
            attributes.insert("wildcard".into(), "true".into());
            metadata.insert("wildcard".into(), "true".into());
        }
        self.edges.push(EdgeRelationship {
            from: code.id,
            to: path,
            edge_type: EdgeType::Imports,
            metadata,
            span: Some(self.span_for(node)),
        });
        self.nodes.push(code);
    }

    /// Records a class/interface/enum, its bases and its Spring injection points. Returns
    /// whether its name was pushed on the class stack.
    fn class(&mut self, node: &Node) -> bool {
        let Some(name_node) = node.child_by_field_name("name") else {
            return false;
        };
        let name = self.node_text(&name_node);
        let node_type = match node.kind() {
            "interface_declaration" => NodeType::Interface,
            _ => NodeType::Class,
        };
        let mut code = CodeNode::new(
            name.clone(),
            Some(node_type),
            Some(Language::Java),
            self.location(node),
        )
        .with_content(self.node_text(node));
        code.span = Some(self.span_for(node));

        let annotations = self.annotations(node);
        let java_kind = if node.kind() == "interface_declaration" {
            "interface"
        } else if modifier_keywords(node).iter().any(|k| k == "abstract") {
            "abstract"
        } else {
            "concrete"
        };
        let attributes = &mut code.metadata.attributes;
        attributes.insert(
            "kind".into(),
            node.kind().trim_end_matches("_declaration").into(),
        );
        attributes.insert("java_kind".into(), java_kind.into());
        if let Some(package) = &self.package {
            attributes.insert("java_package".into(), package.clone());
        }
        if !annotations.is_empty() {
            attributes.insert(crg_shape::DECORATORS_ATTR.into(), annotations.join(","));
        }

        let id = code.id;
        self.push_symbol(&name, &code);
        self.nodes.push(code);
        self.classes.push(name.clone());

        for base in self.bases(node) {
            let java_type = self.java_type(&base);
            self.edges.push(EdgeRelationship {
                from: id,
                to: base,
                edge_type: EdgeType::Extends,
                metadata: HashMap::from([
                    ("source_file".to_string(), self.file_path.to_string()),
                    ("java_type".to_string(), java_type),
                ]),
                span: Some(self.span_for(node)),
            });
        }
        self.injections(node, id, &annotations);
        self.framework_class_edges(node, id, &name);
        true
    }

    /// Records a method or constructor. Returns whether it was pushed on the function stack.
    fn function(&mut self, node: &Node) -> bool {
        let Some(name_node) = node.child_by_field_name("name") else {
            return false;
        };
        let name = self.node_text(&name_node);
        let content_text = self.node_text(node);
        let mut code = CodeNode::new(
            name.clone(),
            Some(NodeType::Function),
            Some(Language::Java),
            self.location(node),
        )
        .with_content(content_text)
        .with_complexity(crate::complexity::calculate_cyclomatic_complexity(
            node,
            self.content,
        ));
        code.span = Some(self.span_for(node));

        let modifiers = modifier_keywords(node);
        let annotations = self.annotations(node);
        let attributes = &mut code.metadata.attributes;
        let kind = if node.kind() == "constructor_declaration" {
            "constructor"
        } else {
            "method"
        };
        attributes.insert("kind".into(), kind.into());
        if let Some(visibility) = modifiers
            .iter()
            .find(|m| matches!(m.as_str(), "public" | "private" | "protected"))
        {
            attributes.insert("visibility".into(), visibility.clone());
        }
        if modifiers.iter().any(|m| m == "static") {
            attributes.insert("static".into(), "true".into());
        }
        if annotations.iter().any(|a| a == "Override") {
            attributes.insert("override".into(), "true".into());
        }
        if let Some(class) = self.classes.last() {
            attributes.insert("parent_class".into(), class.clone());
        }
        if !annotations.is_empty() {
            attributes.insert(crg_shape::DECORATORS_ATTR.into(), annotations.join(","));
        }

        let id = code.id;
        self.push_symbol(&name, &code);
        self.functions.push(id);
        self.nodes.push(code);
        self.framework_method_edges(node, id, &name);
        true
    }

    fn push_symbol(&mut self, name: &str, _node: &CodeNode) {
        let mut path = self.classes.join(".");
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(name);
        self.symbols
            .push((name.to_string(), format!("{}::{path}", self.file_path)));
    }

    fn call(&mut self, node: &Node, bindings: &Bindings, class_fields: &Bindings) {
        let (receiver, method, this_field) = match node.kind() {
            "method_invocation" => {
                let (method, receiver) = self.method_and_receiver(node);
                (receiver, method, this_field_call(node))
            }
            "method_reference" => {
                let named: Vec<Node> = named_children(node).collect();
                match (named.first(), named.last()) {
                    (Some(first), Some(last))
                        if named.len() >= 2
                            && matches!(last.kind(), "identifier" | "type_identifier") =>
                    {
                        (
                            Some(self.node_text(first)),
                            Some(self.node_text(last)),
                            false,
                        )
                    }
                    _ => (None, None, false),
                }
            }
            _ => (None, self.java_type_name(node), false),
        };
        let Some(method) = method.filter(|m| !m.is_empty()) else {
            return;
        };

        let mut metadata = HashMap::from([
            ("call_type".to_string(), "java_invocation".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        if node.kind() == "object_creation_expression" {
            metadata.insert("java_construction".into(), "true".into());
        }
        let mut target = method.clone();
        let expression_receiver =
            node.kind() == "method_invocation" && self.java_expression_receiver(node);
        if expression_receiver {
            metadata.insert("receiver_form".into(), "expression".into());
        }
        let constructed_type = (node.kind() == "method_invocation")
            .then(|| {
                children(node)
                    .next()
                    .filter(|first| first.kind() == "object_creation_expression")
                    .and_then(|first| self.java_type_name(&first))
            })
            .flatten();
        if let Some(receiver) = &receiver {
            metadata.insert("receiver".into(), receiver.clone());
            let declared = if this_field {
                class_fields.get(receiver)
            } else {
                bindings.get(receiver)
            };
            let receiver_type = declared.cloned().or_else(|| {
                let class_receiver = !this_field
                    && receiver.starts_with(|c: char| c.is_ascii_uppercase())
                    && (self.imports.contains_key(receiver)
                        || self.defined.contains(receiver)
                        || package_qualified_prefix(receiver).is_none());
                class_receiver.then(|| receiver.clone())
            });
            if let Some(receiver_type) = receiver_type {
                // CRG keeps the declared type on every typed call, even when the defining
                // file is outside this file; the catalog finishes the binding in that case.
                metadata.insert("receiver_type".into(), receiver_type.clone());
                metadata.insert("receiver_resolution".into(), "typed_receiver".into());
                if let Some(typed) = self.typed_method_target(&receiver_type, &method) {
                    target = typed;
                }
            }
        }
        if receiver.is_none()
            && let Some(receiver_type) = constructed_type
        {
            metadata.insert("receiver_type".into(), receiver_type);
            metadata.insert("receiver_resolution".into(), "constructed_receiver".into());
        }
        let from = match self.functions.last() {
            Some(&id) => id,
            None => {
                metadata.insert(crg_shape::FILE_SCOPE_ATTR.to_string(), "true".to_string());
                NodeId::nil()
            }
        };
        if receiver.is_none() && !expression_receiver {
            self.bare_calls.push(self.edges.len());
        }
        self.edges.push(EdgeRelationship {
            from,
            to: target,
            edge_type: EdgeType::Calls,
            metadata,
            span: Some(self.span_for(node)),
        });
    }

    /// CRG `_get_java_method_and_receiver`: `(method, receiver)` of a `method_invocation`.
    fn method_and_receiver(&self, node: &Node) -> (Option<String>, Option<String>) {
        let kids: Vec<Node> = children(node).collect();
        if kids.len() == 2 && kids[0].kind() == "identifier" && kids[1].kind() == "argument_list" {
            return (Some(self.node_text(&kids[0])), None);
        }
        if kids.len() < 3 {
            return (None, None);
        }
        let mut method = None;
        let mut receiver = None;
        for kid in kids.iter().rev() {
            match kid.kind() {
                "argument_list" | "." => continue,
                "identifier" => {
                    if method.is_none() {
                        method = Some(self.node_text(kid));
                        continue;
                    }
                    receiver = Some(self.node_text(kid));
                    break;
                }
                _ => break,
            }
        }
        if method.is_some() {
            let first = kids[0];
            match first.kind() {
                "identifier" => {
                    let text = self.node_text(&first);
                    if Some(&text) != method.as_ref() {
                        receiver = Some(text);
                    }
                }
                "field_access" => {
                    let owner = first.child_by_field_name("object");
                    if owner.is_some_and(|o| o.kind() == "this")
                        && let Some(field) = first.child_by_field_name("field")
                    {
                        receiver = Some(self.node_text(&field));
                    }
                }
                "this" if kids[1].kind() == "." => receiver = Some("this".into()),
                _ => {}
            }
        }
        (method, receiver)
    }

    /// CRG `_java_expression_receiver`: a call on a chain or literal has no name to type.
    /// It must not be mistaken for a receiver-less call and bound to a file-local symbol.
    fn java_expression_receiver(&self, node: &Node) -> bool {
        let first = children(node).next();
        first.is_some_and(|first| {
            matches!(
                first.kind(),
                "method_invocation"
                    | "array_access"
                    | "string_literal"
                    | "character_literal"
                    | "null_literal"
                    | "parenthesized_expression"
            )
        })
    }

    /// CRG `_resolve_typed_method_target` for a type declared in this file: `<file>::Type.m`.
    /// Package-qualified and other-file types are left to the catalog.
    fn typed_method_target(&self, type_name: &str, method: &str) -> Option<String> {
        if package_qualified_prefix(type_name).is_some_and(|q| q.len() == type_name.len()) {
            return None;
        }
        let base = base_type_name(type_name)?;
        self.defined
            .contains(&base)
            .then(|| format!("{}::{base}.{method}", self.file_path))
    }

    /// Receiver-less calls bind to the first same-named class or method of the file, as
    /// CRG's `_resolve_call_targets` does (`super.save` -> the first `save`).
    fn bind_bare_calls(&mut self) {
        for &index in &self.bare_calls {
            let edge = &mut self.edges[index];
            if edge.to.contains("::") {
                continue;
            }
            if let Some((_, qualified)) = self.symbols.iter().find(|(name, _)| *name == edge.to) {
                edge.to = qualified.clone();
            }
        }
    }

    /// CRG `_collect_class_typed_fields`: declared types inside a class, not entering nested
    /// classes or methods.
    fn class_typed_fields(&self, class: &Node) -> Bindings {
        let mut fields = Bindings::new();
        let mut stack = vec![*class];
        while let Some(node) = stack.pop() {
            if node.id() != class.id() && (is_class(node.kind()) || is_function(node.kind())) {
                continue;
            }
            fields.extend(self.typed_bindings(&node));
            let mut kids: Vec<Node> = children(&node).collect();
            kids.reverse();
            stack.extend(kids);
        }
        fields
    }

    /// CRG `_collect_function_typed_parameters`.
    fn typed_parameters(&self, function: &Node) -> Bindings {
        let mut bindings = Bindings::new();
        let mut stack = vec![*function];
        while let Some(node) = stack.pop() {
            if node.id() != function.id() && is_function(node.kind()) {
                continue;
            }
            if matches!(node.kind(), "formal_parameter" | "spread_parameter") {
                bindings.extend(self.typed_bindings(&node));
                continue;
            }
            if node.kind() == "block" {
                continue;
            }
            let mut kids: Vec<Node> = children(&node).collect();
            kids.reverse();
            stack.extend(kids);
        }
        bindings
    }

    /// CRG `_typed_bindings_from_node` for Java.
    fn typed_bindings(&self, node: &Node) -> Bindings {
        let mut result = Bindings::new();
        match node.kind() {
            "formal_parameter" | "spread_parameter" => self.store_binding(
                &mut result,
                node.child_by_field_name("name"),
                node.child_by_field_name("type"),
            ),
            "field_declaration" | "local_variable_declaration" => {
                let type_node = node.child_by_field_name("type");
                for declarator in children(node).filter(|c| c.kind() == "variable_declarator") {
                    let mut declared = type_node;
                    if let Some(ty) = type_node
                        && self.node_text(&ty) == "var"
                        && let Some(value) = declarator.child_by_field_name("value")
                        && value.kind() == "object_creation_expression"
                    {
                        // `var x = new Cart()` states its type on the right.
                        declared = value.child_by_field_name("type").or(Some(ty));
                    }
                    self.store_binding(
                        &mut result,
                        declarator.child_by_field_name("name"),
                        declared,
                    );
                }
            }
            _ => {}
        }
        result
    }

    /// CRG `_store_java_typed_binding`: a package-qualified type is kept whole.
    fn store_binding(&self, result: &mut Bindings, name: Option<Node>, ty: Option<Node>) {
        let (Some(name), Some(ty)) = (name, ty) else {
            return;
        };
        let name = self.node_text(&name);
        let type_text = self.node_text(&ty);
        let Some(base) = base_type_name(&type_text) else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let stored = match package_qualified_prefix(type_text.trim()) {
            Some(qualified) if qualified.rsplit('.').next() == Some(base.as_str()) => {
                qualified.to_string()
            }
            _ => base,
        };
        result.insert(name, stored);
    }

    /// CRG `_java_type_name`: the first type identifier under a node, depth first.
    fn java_type_name(&self, node: &Node) -> Option<String> {
        if matches!(node.kind(), "type_identifier" | "scoped_type_identifier") {
            return Some(self.node_text(node));
        }
        children(node).find_map(|child| self.java_type_name(&child))
    }

    /// CRG `_get_bases` for Java: `extends` and `implements` types as written.
    fn bases(&self, node: &Node) -> Vec<String> {
        let mut bases = Vec::new();
        for child in children(node) {
            let types: Vec<Node> = match child.kind() {
                "superclass" => children(&child).collect(),
                "super_interfaces" => children(&child)
                    .filter(|c| c.kind() == "type_list")
                    .flat_map(|list| children(&list).collect::<Vec<_>>())
                    .collect(),
                _ => continue,
            };
            bases.extend(
                types
                    .iter()
                    .filter(|t| matches!(t.kind(), "type_identifier" | "generic_type"))
                    .map(|t| self.node_text(t)),
            );
        }
        bases
    }

    /// CRG `_emit_spring_injections`: one INJECTS edge per injected field or `@Autowired`
    /// constructor parameter.
    fn injections(&mut self, class: &Node, class_id: NodeId, annotations: &[String]) {
        let has_required_args = annotations.iter().any(|a| a == "RequiredArgsConstructor");
        let has_all_args = annotations.iter().any(|a| a == "AllArgsConstructor");
        let Some(body) = children(class).find(|c| c.kind() == "class_body") else {
            return;
        };
        for member in children(&body) {
            match member.kind() {
                "field_declaration" => {
                    self.field_injection(&member, class_id, has_required_args, has_all_args)
                }
                "constructor_declaration" => self.constructor_injection(&member, class_id),
                _ => {}
            }
        }
    }

    fn field_injection(
        &mut self,
        field: &Node,
        class_id: NodeId,
        has_required_args: bool,
        has_all_args: bool,
    ) {
        let mut annotations = Vec::new();
        let (mut is_final, mut is_static) = (false, false);
        let mut field_type = None;
        let mut declarators = Vec::new();
        for child in children(field) {
            match child.kind() {
                "modifiers" => {
                    for modifier in children(&child) {
                        match self.node_text(&modifier).as_str() {
                            "final" => is_final = true,
                            "static" => is_static = true,
                            _ if matches!(modifier.kind(), "marker_annotation" | "annotation") => {
                                if let Some(name) =
                                    children(&modifier).find(|c| c.kind() == "identifier")
                                {
                                    annotations.push(self.node_text(&name));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "type_identifier" => field_type = Some(self.node_text(&child)),
                "generic_type" | "array_type" => {
                    if let Some(inner) = children(&child).find(|c| c.kind() == "type_identifier") {
                        field_type = Some(self.node_text(&inner));
                    }
                }
                "variable_declarator" => {
                    if let Some(name) = children(&child).find(|c| c.kind() == "identifier") {
                        let initialized = child.child_by_field_name("value").is_some();
                        declarators.push((self.node_text(&name), initialized));
                    }
                }
                _ => {}
            }
        }
        let Some(field_type) = field_type else {
            return;
        };
        if is_static {
            return;
        }
        let injected = annotations
            .iter()
            .any(|a| INJECT_ANNOTATIONS.contains(&a.as_str()));
        let non_null = annotations.iter().any(|a| a == "NonNull");
        for (field_name, initialized) in declarators {
            let injection_type = if injected {
                "field"
            } else if has_all_args {
                "constructor_lombok_all"
            } else if has_required_args && !initialized && (is_final || non_null) {
                "constructor_lombok"
            } else {
                continue;
            };
            self.push_injection(
                class_id,
                &field_type,
                injection_type,
                Some(field_name),
                field,
            );
        }
    }

    fn constructor_injection(&mut self, constructor: &Node, class_id: NodeId) {
        let annotations = self.annotations(constructor);
        if !annotations
            .iter()
            .any(|a| INJECT_ANNOTATIONS.contains(&a.as_str()))
        {
            return;
        }
        let parameters: Vec<Node> = children(constructor)
            .filter(|c| c.kind() == "formal_parameters")
            .flat_map(|list| children(&list).collect::<Vec<_>>())
            .filter(|p| p.kind() == "formal_parameter")
            .collect();
        for parameter in parameters {
            let mut param_type = None;
            let mut param_name = None;
            for sub in children(&parameter) {
                match sub.kind() {
                    "type_identifier" if param_type.is_none() => {
                        param_type = Some(self.node_text(&sub))
                    }
                    "identifier" => param_name = Some(self.node_text(&sub)),
                    _ => {}
                }
            }
            if let Some(param_type) = param_type {
                self.push_injection(class_id, &param_type, "constructor", param_name, &parameter);
            }
        }
    }

    fn push_injection(
        &mut self,
        class_id: NodeId,
        injected: &str,
        injection_type: &str,
        field_name: Option<String>,
        node: &Node,
    ) {
        let mut metadata = HashMap::from([
            ("source_file".to_string(), self.file_path.to_string()),
            ("injection_type".to_string(), injection_type.to_string()),
            ("java_type".to_string(), self.java_type(injected)),
        ]);
        if let Some(field_name) = field_name {
            metadata.insert("field_name".into(), field_name);
        }
        self.edges.push(EdgeRelationship {
            from: class_id,
            to: injected.to_string(),
            edge_type: EdgeType::Other(INJECTS.into()),
            metadata,
            span: Some(self.span_for(node)),
        });
    }

    /// CRG `_spring_java_type`: package-qualified identity of a type, bare when a wildcard
    /// import hides its package.
    fn java_type(&self, type_name: &str) -> String {
        let normalized = type_name.trim();
        let (head, tail) = match normalized.split_once('.') {
            Some((head, tail)) => (head, Some(tail)),
            None => (normalized, None),
        };
        if self.wildcard_import && !self.imports.contains_key(head) {
            return normalized.to_string();
        }
        if normalized.is_empty() {
            return String::new();
        }
        if let Some(imported) = self.imports.get(head) {
            return match tail {
                Some(tail) => format!("{imported}.{tail}"),
                None => imported.clone(),
            };
        }
        if tail.is_some() && head.starts_with(|c: char| c.is_lowercase()) {
            return normalized.to_string();
        }
        match &self.package {
            Some(package) => format!("{package}.{normalized}"),
            None => normalized.to_string(),
        }
    }

    /// Simple names of the annotations in a declaration's modifiers.
    fn annotations(&self, node: &Node) -> Vec<String> {
        children(node)
            .filter(|c| c.kind() == "modifiers")
            .flat_map(|m| children(&m).collect::<Vec<_>>())
            .filter(|m| matches!(m.kind(), "marker_annotation" | "annotation"))
            .filter_map(|m| {
                children(&m).find_map(|c| match c.kind() {
                    "identifier" => Some(self.node_text(&c)),
                    "scoped_identifier" => Some(self.node_text(&c)),
                    _ => None,
                })
            })
            .collect()
    }

    /// Annotation nodes of one declaration, in source order.
    fn annotation_nodes<'t>(&self, node: &Node<'t>) -> Vec<Node<'t>> {
        children(node)
            .filter(|c| c.kind() == "modifiers")
            .flat_map(|modifiers| children(&modifiers).collect::<Vec<_>>())
            .filter(|c| matches!(c.kind(), "marker_annotation" | "annotation"))
            .collect()
    }

    /// Simple name of an annotation, even when written with a package prefix.
    fn annotation_name(&self, annotation: &Node) -> Option<String> {
        children(annotation).find_map(|child| match child.kind() {
            "identifier" => Some(self.node_text(&child)),
            "scoped_identifier" => self
                .node_text(&child)
                .rsplit('.')
                .next()
                .map(str::to_string),
            _ => None,
        })
    }

    /// String literals under a node, in depth-first source order.
    fn string_literals(&self, node: &Node) -> Vec<String> {
        let mut values = Vec::new();
        let mut stack = vec![*node];
        while let Some(current) = stack.pop() {
            if current.kind() == "string_literal" {
                values.push(
                    self.node_text(&current)
                        .trim()
                        .trim_matches('"')
                        .to_string(),
                );
            }
            let mut children: Vec<Node> = children(&current).collect();
            children.reverse();
            stack.extend(children);
        }
        values
    }

    /// Named and positional annotation arguments, kept as strings for framework edges.
    fn annotation_values(&self, annotation: &Node) -> HashMap<String, Vec<String>> {
        let mut values: HashMap<String, Vec<String>> = HashMap::new();
        let mut stack = vec![*annotation];
        while let Some(current) = stack.pop() {
            if current.kind() == "element_value_pair" {
                let key = current
                    .child_by_field_name("key")
                    .or_else(|| children(&current).find(|child| child.kind() == "identifier"));
                let value = current
                    .child_by_field_name("value")
                    .or_else(|| children(&current).nth(1));
                if let (Some(key), Some(value)) = (key, value) {
                    let strings = self.string_literals(&value);
                    if !strings.is_empty() {
                        values
                            .entry(self.node_text(&key))
                            .or_default()
                            .extend(strings);
                    }
                }
            }
            let mut children: Vec<Node> = children(&current).collect();
            children.reverse();
            stack.extend(children);
        }
        if values.is_empty() {
            let positional = self.string_literals(annotation);
            if !positional.is_empty() {
                values.insert("value".into(), positional);
            }
        }
        values
    }

    fn annotation_value(&self, annotation: &Node, keys: &[&str]) -> Option<String> {
        let values = self.annotation_values(annotation);
        keys.iter()
            .find_map(|key| values.get(*key))
            .and_then(|found| found.first())
            .cloned()
    }

    /// `${key:default}` keys, without the default, as CRG's config emitter keeps them.
    fn config_placeholders(&self, value: &str) -> Vec<String> {
        let mut keys = Vec::new();
        let bytes = value.as_bytes();
        let mut index = 0;
        while let Some(start) = value[index..].find("${").map(|offset| index + offset + 2) {
            let Some(end) = bytes[start..]
                .iter()
                .position(|byte| *byte == b'}')
                .map(|offset| start + offset)
            else {
                break;
            };
            let key = value[start..end]
                .split_once(':')
                .map(|(key, _)| key)
                .unwrap_or(&value[start..end])
                .trim();
            if !key.is_empty() {
                keys.push(key.to_string());
            }
            index = end + 1;
        }
        keys
    }

    fn first_parameter_type(&self, function: &Node) -> Option<String> {
        children(function)
            .find(|child| child.kind() == "formal_parameters")
            .and_then(|parameters| {
                children(&parameters).find(|child| child.kind() == "formal_parameter")
            })
            .and_then(|parameter| parameter.child_by_field_name("type"))
            .map(|type_node| self.node_text(&type_node))
    }

    /// Type identities in `Foo.class` values or, failing that, the first parameter.
    fn event_types(&self, annotation: &Node, function: &Node) -> Vec<String> {
        let mut types = Vec::new();
        let mut stack = vec![*annotation];
        while let Some(current) = stack.pop() {
            if current.kind() == "class_literal"
                && let Some(name) = self.java_type_name(&current)
            {
                types.push(name);
            }
            let mut children: Vec<Node> = children(&current).collect();
            children.reverse();
            stack.extend(children);
        }
        if types.is_empty()
            && let Some(name) = self.first_parameter_type(function)
        {
            types.push(name);
        }
        types
            .into_iter()
            .map(|name| self.java_type(&name))
            .filter(|identity| !identity.is_empty())
            .collect()
    }

    fn http_method(&self, annotation: &Node, name: &str) -> Option<String> {
        if !REQUEST_MAPPINGS.contains(&name) {
            return None;
        }
        let method = match name {
            "GetMapping" => "GET",
            "PostMapping" => "POST",
            "PutMapping" => "PUT",
            "DeleteMapping" => "DELETE",
            "PatchMapping" => "PATCH",
            "RequestMapping" => &self
                .annotation_value(annotation, &["method"])
                .filter(|value| !value.is_empty())?,
            _ => return None,
        };
        Some(method.to_string())
    }

    /// A synthetic CRG node (Endpoint/Scheduler/Event), with the qualified name the edge names.
    fn framework_node(
        &mut self,
        name: String,
        kind: &str,
        qualified: String,
        file_path: String,
        line: u32,
        parent: Option<&str>,
        metadata: HashMap<String, String>,
    ) -> NodeId {
        let mut node = CodeNode::new(
            name,
            Some(NodeType::Other(kind.into())),
            Some(Language::Java),
            Location {
                file_path,
                line,
                column: 0,
                end_line: Some(line),
                end_column: Some(0),
            },
        );
        let attributes = &mut node.metadata.attributes;
        attributes.insert("qualified_name".into(), qualified);
        if let Some(parent) = parent {
            attributes.insert("parent_name".into(), parent.to_string());
        }
        attributes.extend(metadata);
        let id = node.id;
        self.nodes.push(node);
        id
    }

    fn event_node(&mut self, event_type: &str) -> NodeId {
        let identity = format!("event::{event_type}");
        if let Some(&id) = self.virtual_events.get(&identity) {
            return id;
        }
        let name = event_type
            .rsplit('.')
            .next()
            .unwrap_or(event_type)
            .to_string();
        let metadata = HashMap::from([
            ("event_type".to_string(), event_type.to_string()),
            ("virtual".to_string(), "true".to_string()),
        ]);
        let id = self.framework_node(
            name,
            "Event",
            identity.clone(),
            "event".to_string(),
            0,
            None,
            metadata,
        );
        self.virtual_events.insert(identity, id);
        id
    }

    fn framework_edge(
        &mut self,
        from: NodeId,
        kind: &str,
        to: String,
        mut metadata: HashMap<String, String>,
        node: &Node,
    ) {
        metadata.insert("source_file".to_string(), self.file_path.to_string());
        metadata.insert(
            "line".to_string(),
            (node.start_position().row + 1).to_string(),
        );
        self.edges.push(EdgeRelationship {
            from,
            to,
            edge_type: EdgeType::Other(kind.into()),
            metadata,
            span: Some(self.span_for(node)),
        });
    }

    fn qualified_here(&self, symbol: &str) -> String {
        let owner = self.classes.join(".");
        if owner.is_empty() {
            format!("{}::{symbol}", self.file_path)
        } else {
            format!("{}::{owner}.{symbol}", self.file_path)
        }
    }

    /// Class-level framework edges: config keys, Temporal stubs and Kafka field templates.
    fn framework_class_edges(&mut self, node: &Node, class_id: NodeId, _class_name: &str) {
        let class_annotations = self.annotation_nodes(node);
        for annotation in &class_annotations {
            if self.annotation_name(&annotation).as_deref() != Some("ConfigurationProperties") {
                continue;
            }
            let Some(prefix) = self.annotation_value(&annotation, &["prefix", "value"]) else {
                continue;
            };
            let key = prefix.trim();
            if key.is_empty() {
                continue;
            }
            self.framework_edge(
                class_id,
                DEPENDS_ON_CONFIG,
                format!("config:{key}.*"),
                HashMap::from([
                    ("config_key".to_string(), key.to_string()),
                    (
                        "resolution".to_string(),
                        "configuration_properties".to_string(),
                    ),
                ]),
                &annotation,
            );
        }
        let Some(body) = children(node).find(|child| child.kind() == "class_body") else {
            return;
        };
        for member in children(&body).filter(|child| child.kind() == "field_declaration") {
            let static_field = children(&member)
                .find(|child| child.kind() == "modifiers")
                .is_some_and(|modifiers| {
                    modifier_keywords(&modifiers)
                        .iter()
                        .any(|keyword| keyword == "static")
                });
            if static_field {
                continue;
            }
            let field_type = children(&member).find_map(|child| match child.kind() {
                "type_identifier" => Some(self.node_text(&child)),
                "generic_type" => children(&child)
                    .find(|inner| inner.kind() == "type_identifier")
                    .map(|inner| self.node_text(&inner)),
                _ => None,
            });
            let field_name = children(&member)
                .find(|child| child.kind() == "variable_declarator")
                .and_then(|declarator| {
                    children(&declarator)
                        .find(|child| child.kind() == "identifier")
                        .map(|child| self.node_text(&child))
                });
            let (Some(field_type), Some(field_name)) = (field_type, field_name) else {
                continue;
            };
            if field_type.ends_with("Activity") || field_type.ends_with("Workflow") {
                self.framework_edge(
                    class_id,
                    TEMPORAL_STUB,
                    field_type.clone(),
                    HashMap::from([
                        ("field_name".to_string(), field_name.clone()),
                        (
                            "stub_type".to_string(),
                            if field_type.ends_with("Activity") {
                                "activity".to_string()
                            } else {
                                "workflow".to_string()
                            },
                        ),
                    ]),
                    &member,
                );
            }
            let kafka_kind = if KAFKA_CONSUMER_TYPES.contains(&field_type.as_str()) {
                Some(CONSUMES)
            } else {
                KAFKA_PRODUCER_TYPES
                    .contains(&field_type.as_str())
                    .then_some(PRODUCES)
            };
            if let Some(kind) = kafka_kind {
                let mut metadata = HashMap::from([
                    ("field_name".to_string(), field_name),
                    ("kafka_type".to_string(), field_type),
                ]);
                let generics: Vec<String> = children(&member)
                    .filter(|child| child.kind() == "generic_type")
                    .flat_map(|generic| self.string_literals(&generic))
                    .collect();
                if let Some(message_type) = generics.last() {
                    metadata.insert("message_type".into(), message_type.clone());
                }
                self.framework_edge(
                    class_id,
                    kind,
                    "kafka:config".to_string(),
                    metadata,
                    &member,
                );
            }
            let field_annotations = self.annotation_nodes(&member);
            for annotation in &field_annotations {
                if self.annotation_name(annotation).as_deref() != Some("Value") {
                    continue;
                }
                for value in self.annotation_values(&annotation).values().flatten() {
                    for key in self.config_placeholders(value) {
                        self.framework_edge(
                            class_id,
                            DEPENDS_ON_CONFIG,
                            format!("config:{key}"),
                            HashMap::from([
                                ("config_key".to_string(), key.clone()),
                                ("resolution".to_string(), "value_annotation".to_string()),
                            ]),
                            &annotation,
                        );
                    }
                }
            }
        }
    }

    /// Method-level framework edges: listeners, mappings, schedules, events and WebFlux routes.
    fn framework_method_edges(&mut self, node: &Node, function_id: NodeId, method_name: &str) {
        let direct = self.annotation_nodes(node);
        let mut mapping_index = 0;
        let mut emitted = 0;
        for annotation in &direct {
            let Some(name) = self.annotation_name(annotation) else {
                continue;
            };
            if KAFKA_LISTENER_ANNOTATIONS.contains(&name.as_str()) {
                let values = self.annotation_values(annotation);
                let topics: Vec<String> = values
                    .get("topics")
                    .cloned()
                    .unwrap_or_else(|| values.get("value").cloned().unwrap_or_default());
                if topics.is_empty() {
                    self.framework_edge(
                        function_id,
                        CONSUMES,
                        "kafka:config".to_string(),
                        HashMap::from([("kafka_type".to_string(), name.clone())]),
                        node,
                    );
                } else {
                    for topic in topics {
                        self.framework_edge(
                            function_id,
                            CONSUMES,
                            format!("kafka:{topic}"),
                            HashMap::from([
                                ("topic".to_string(), topic.clone()),
                                ("kafka_type".to_string(), "KafkaListener".to_string()),
                            ]),
                            node,
                        );
                    }
                }
            }
            if let Some(http_method) = self.http_method(annotation, &name) {
                let route = self
                    .annotation_value(annotation, &["path", "value"])
                    .map(|route| {
                        if route.starts_with('/') {
                            route
                        } else {
                            format!("/{route}")
                        }
                    })
                    .unwrap_or_else(|| "/".to_string());
                let endpoint_name = format!(
                    "{method_name}@{name}[{mapping_index}:{emitted}] {http_method} {route}"
                );
                let metadata = HashMap::from([
                    ("annotation".to_string(), name.clone()),
                    ("handler".to_string(), method_name.to_string()),
                    ("http_method".to_string(), http_method.clone()),
                    ("route".to_string(), route.clone()),
                ]);
                let qualified = self.qualified_here(&endpoint_name);
                let owner = self.classes.last().cloned();
                self.framework_node(
                    endpoint_name,
                    "Endpoint",
                    qualified.clone(),
                    self.file_path.to_string(),
                    (annotation.start_position().row + 1) as u32,
                    owner.as_deref(),
                    metadata.clone(),
                );
                self.framework_edge(function_id, HANDLES, qualified, metadata, annotation);
                emitted += 1;
                mapping_index += 1;
            }
            if name == "Scheduled" || name == "Schedules" {
                let values = self.annotation_values(annotation);
                let schedule_kind = [
                    "cron",
                    "fixedRate",
                    "fixedRateString",
                    "fixedDelay",
                    "fixedDelayString",
                    "initialDelay",
                    "initialDelayString",
                ]
                .iter()
                .find(|key| values.contains_key(**key))
                .copied()
                .unwrap_or("scheduled");
                let schedule_name =
                    format!("{method_name}@Scheduled[{mapping_index}]:{schedule_kind}");
                let mut metadata = HashMap::from([
                    ("annotation".to_string(), "Scheduled".to_string()),
                    ("schedule_kind".to_string(), schedule_kind.to_string()),
                ]);
                if let Some(value) = values.get(schedule_kind).and_then(|v| v.first()) {
                    metadata.insert(schedule_kind.to_string(), value.clone());
                }
                let qualified = self.qualified_here(&schedule_name);
                let owner = self.classes.last().cloned();
                let scheduler = self.framework_node(
                    schedule_name,
                    "Scheduler",
                    qualified,
                    self.file_path.to_string(),
                    (annotation.start_position().row + 1) as u32,
                    owner.as_deref(),
                    metadata.clone(),
                );
                self.framework_edge(
                    scheduler,
                    TRIGGERS,
                    self.qualified_here(method_name),
                    metadata,
                    annotation,
                );
                mapping_index += 1;
            }
            if name == "EventListener" {
                for identity in self.event_types(annotation, node) {
                    self.framework_edge(
                        function_id,
                        HANDLES,
                        format!("event::{identity}"),
                        HashMap::from([
                            ("event_type".to_string(), identity.clone()),
                            (
                                "resolution".to_string(),
                                "spring_event_listener".to_string(),
                            ),
                        ]),
                        annotation,
                    );
                    self.event_node(&identity);
                }
            }
        }
        self.publish_event_edges(node, function_id);
    }

    /// `publishEvent(new Event())` edges, with the event type CRG resolves from the import map.
    fn publish_event_edges(&mut self, node: &Node, function_id: NodeId) {
        let mut stack = vec![*node];
        while let Some(current) = stack.pop() {
            if current.kind() == "method_invocation" {
                let identifiers: Vec<Node> = children(&current)
                    .filter(|child| child.kind() == "identifier")
                    .collect();
                let method = identifiers.last().map(|child| self.node_text(child));
                if method.as_deref() == Some("publishEvent") {
                    let mut created_nodes = Vec::new();
                    let mut stack = vec![current];
                    while let Some(node) = stack.pop() {
                        if node.kind() == "object_creation_expression" {
                            created_nodes.push(node);
                        }
                        let mut descendants: Vec<Node> = children(&node).collect();
                        descendants.reverse();
                        stack.extend(descendants);
                    }
                    for created in created_nodes {
                        if let Some(name) = self.java_type_name(&created) {
                            let identity = self.java_type(&name);
                            self.framework_edge(
                                function_id,
                                PUBLISHES,
                                format!("event::{identity}"),
                                HashMap::from([
                                    ("event_type".to_string(), identity.clone()),
                                    ("resolution".to_string(), "spring_publish_event".to_string()),
                                    ("receiver".to_string(), "publisher".to_string()),
                                ]),
                                &created,
                            );
                            self.event_node(&identity);
                        }
                    }
                }
            }
            let mut children: Vec<Node> = children(&current).collect();
            children.reverse();
            stack.extend(children);
        }
    }

    fn span_for(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
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

fn is_class(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "interface_declaration" | "enum_declaration"
    )
}

fn is_function(kind: &str) -> bool {
    matches!(kind, "method_declaration" | "constructor_declaration")
}

fn children<'t>(node: &Node<'t>) -> impl Iterator<Item = Node<'t>> + use<'t> {
    let node = *node;
    (0..node.child_count()).filter_map(move |i| node.child(i))
}

fn named_children<'t>(node: &Node<'t>) -> impl Iterator<Item = Node<'t>> + use<'t> {
    let node = *node;
    (0..node.named_child_count() as u32).filter_map(move |i| node.named_child(i))
}

/// Keyword modifiers (`public`, `static`, `abstract`, ...) of a declaration.
fn modifier_keywords(node: &Node) -> Vec<String> {
    children(node)
        .filter(|c| c.kind() == "modifiers")
        .flat_map(|m| children(&m).collect::<Vec<_>>())
        .filter(|m| !m.is_named())
        .map(|m| m.kind().to_string())
        .collect()
}

/// CRG `_java_this_field_call`: a call made on `this.field`.
fn this_field_call(node: &Node) -> bool {
    node.child_by_field_name("object").is_some_and(|receiver| {
        receiver.kind() == "field_access"
            && receiver
                .child_by_field_name("object")
                .is_some_and(|owner| owner.kind() == "this")
    })
}

/// The leading match of CRG `_JAVA_PACKAGE_QUALIFIED_TYPE` (`com.acme.Cart`), if any.
fn package_qualified_prefix(text: &str) -> Option<&str> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    // Length of the package segments read so far, dots included.
    let mut end = 0;
    for segment in text.split('.') {
        let length = segment.find(|c: char| !is_word(c)).unwrap_or(segment.len());
        let first = segment.chars().next()?;
        if first.is_ascii_uppercase() {
            return (end > 0 && length > 0).then(|| &text[..end + length]);
        }
        let package_start = first.is_ascii_lowercase() || first == '_' || first == '$';
        if !package_start || length != segment.len() {
            return None;
        }
        end += segment.len() + 1;
    }
    None
}

/// CRG `_base_type_name`: the receiver class of a declared type, `None` for containers,
/// arrays and scalars. `Optional<T>` (and the other transparent wrappers) unwrap to `T`.
fn base_type_name(annotation: &str) -> Option<String> {
    let mut current = annotation.trim();
    for _ in 0..4 {
        let start = current.find(|c: char| c.is_ascii_alphabetic() || c == '_')?;
        let rest = &current[start..];
        let length = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
            .unwrap_or(rest.len());
        let outer = rest[..length].rsplit('.').next().unwrap_or("");
        let remainder = rest[length..].trim_start();
        if remainder.starts_with("[]") {
            return None;
        }
        if TRANSPARENT_WRAPPERS.contains(&outer) {
            let opening = current.find(['[', '<'])?;
            current = current[opening + 1..].trim_start();
            continue;
        }
        if NON_RECEIVER_TYPES.contains(&outer) || SCALAR_TYPES.contains(&outer) {
            return None;
        }
        return Some(outer.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{base_type_name, package_qualified_prefix};
    use codegraph_core::{EdgeType, ExtractionResult, Language, NodeType};
    use std::collections::BTreeSet;

    fn extract_java(source: &str, path: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        crate::languages::extract_for_language_at(
            &Language::Java,
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

    fn node<'r>(
        result: &'r ExtractionResult,
        qualified_name: &str,
    ) -> &'r codegraph_core::CodeNode {
        result
            .nodes
            .iter()
            .find(|n| {
                n.metadata
                    .attributes
                    .get("qualified_name")
                    .map(String::as_str)
                    == Some(qualified_name)
            })
            .unwrap_or_else(|| panic!("no node {qualified_name}"))
    }

    /// CRG `tests/fixtures/SampleJava.java`, without comments.
    const SAMPLE: &str = r#"package com.example.auth;

import java.util.HashMap;
import java.util.Map;
import java.util.Optional;

public interface UserRepository {
    Optional<User> findById(int id);
    void save(User user);
}

class User {
    private int id;
    private String name;
    private String email;

    public User(int id, String name, String email) {
        this.id = id;
        this.name = name;
        this.email = email;
    }

    public int getId() { return id; }
    public String getName() { return name; }
    public String getEmail() { return email; }
}

class InMemoryRepo implements UserRepository {
    private Map<Integer, User> users = new HashMap<>();

    @Override
    public Optional<User> findById(int id) {
        return Optional.ofNullable(users.get(id));
    }

    @Override
    public void save(User user) {
        users.put(user.getId(), user);
        System.out.println("Saved user " + user.getId());
    }
}

class UserService {
    private final UserRepository repo;

    public UserService(UserRepository repo) {
        this.repo = repo;
    }

    public User createUser(String name, String email) {
        User user = new User(1, name, email);
        repo.save(user);
        return user;
    }

    public Optional<User> getUser(int id) {
        return repo.findById(id);
    }
}

class CachedRepo extends InMemoryRepo {
    @Override
    public void save(User user) {
        super.save(user);
    }
}
"#;

    /// CRG `tests/fixtures/SpringDI.java`, without comments.
    const SPRING: &str = r#"package com.example.shop;

import org.springframework.stereotype.Service;
import org.springframework.stereotype.Repository;
import org.springframework.stereotype.Component;
import org.springframework.beans.factory.annotation.Autowired;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
import lombok.RequiredArgsConstructor;

public interface OrderRepository {
    void save(Order order);
    Order findById(Long id);
}

@Repository
class JpaOrderRepository implements OrderRepository {
    @Override
    public void save(Order order) {}

    @Override
    public Order findById(Long id) { return null; }
}

@Service
class NotificationService {
    @Autowired
    private OrderRepository orderRepository;

    public void notify(Long orderId) {
        Order o = orderRepository.findById(orderId);
    }
}

@Service
@RequiredArgsConstructor
class OrderService {
    private final OrderRepository orderRepository;
    private final NotificationService notificationService;
    private static final String TAG = "OrderService";

    public void placeOrder(Order order) {
        orderRepository.save(order);
        notificationService.notify(order.getId());
    }
}

@Component
class AuditLogger {
    private final OrderRepository orderRepository;

    @Autowired
    public AuditLogger(OrderRepository orderRepository) {
        this.orderRepository = orderRepository;
    }

    public void log(String msg) {}
}

@Configuration
class AppConfig {
    @Bean
    public OrderRepository orderRepository() {
        return new JpaOrderRepository();
    }
}

class Order {
    private Long id;
    public Long getId() { return id; }
}
"#;

    #[test]
    fn package_is_an_attribute_not_a_node() {
        let result = extract_java(SAMPLE, "S.java");
        assert!(
            result
                .nodes
                .iter()
                .all(|n| n.name.as_str() != "com.example.auth")
        );
        let user = node(&result, "S.java::User");
        assert_eq!(
            user.metadata
                .attributes
                .get("java_package")
                .map(String::as_str),
            Some("com.example.auth")
        );
        assert_eq!(
            user.metadata
                .attributes
                .get("java_kind")
                .map(String::as_str),
            Some("concrete")
        );
        let repo = node(&result, "S.java::UserRepository");
        assert_eq!(
            repo.metadata
                .attributes
                .get("java_kind")
                .map(String::as_str),
            Some("interface")
        );
    }

    #[test]
    fn enums_are_classes_and_abstract_is_recorded() {
        let source = "enum Color implements Named { RED; void paint() { mix(); } }\n\
                      abstract class Shape { }\n";
        let result = extract_java(source, "E.java");
        assert_eq!(
            node(&result, "E.java::Color").node_type,
            Some(NodeType::Class)
        );
        assert_eq!(
            node(&result, "E.java::Shape")
                .metadata
                .attributes
                .get("java_kind")
                .map(String::as_str),
            Some("abstract")
        );
        assert!(
            edges(&result, EdgeType::Extends).contains(&("E.java::Color".into(), "Named".into()))
        );
        assert_eq!(calls_from(&result, "E.java::Color.paint"), set(&["mix"]));
    }

    #[test]
    fn calls_take_crg_names_and_callers() {
        let source = "class A {\n\
                      private Map<Integer, User> users = new HashMap<>();\n\
                      void one() { helper(); }\n\
                      void two() { list.forEach(Foo::bar); x.<T>gen(); }\n\
                      }\n";
        let result = extract_java(source, "A.java");
        assert_eq!(calls_from(&result, "A.java"), set(&["HashMap"]));
        assert_eq!(calls_from(&result, "A.java::A.one"), set(&["helper"]));
        assert_eq!(
            calls_from(&result, "A.java::A.two"),
            set(&["forEach", "bar", "gen"])
        );
    }

    #[test]
    fn sample_java_calls_match_crg() {
        let result = extract_java(SAMPLE, "S.java");
        let s = |name: &str| format!("S.java::{name}");
        assert_eq!(
            calls_from(&result, &s("CachedRepo.save")),
            set(&["S.java::UserRepository.save"])
        );
        assert_eq!(
            calls_from(&result, &s("InMemoryRepo.findById")),
            set(&["get", "ofNullable"])
        );
        assert_eq!(
            calls_from(&result, &s("InMemoryRepo.save")),
            set(&["S.java::User.getId", "println", "put"])
        );
        assert_eq!(
            calls_from(&result, &s("UserService.createUser")),
            set(&["S.java::User", "S.java::UserRepository.save"])
        );
        assert_eq!(
            calls_from(&result, &s("UserService.getUser")),
            set(&["S.java::UserRepository.findById"])
        );
        assert_eq!(calls_from(&result, "S.java"), set(&["HashMap"]));
    }

    #[test]
    fn receivers_follow_declared_types() {
        let source = "class Cart { void total() {} static void make() {} }\n\
                      class Shop {\n\
                      private Cart cart;\n\
                      private Optional<Cart> maybe;\n\
                      private List<Cart> carts;\n\
                      private Other other;\n\
                      void a(Other cart) { cart.total(); this.cart.total(); }\n\
                      void b() { var c = new Cart(); c.total(); maybe.total(); }\n\
                      void c() { carts.total(); other.total(); Cart.make(); }\n\
                      void d() { com.acme.Cart q = null; q.total(); }\n\
                      }\n";
        let result = extract_java(source, "T.java");
        assert_eq!(
            calls_from(&result, "T.java::Shop.a"),
            set(&["total", "T.java::Cart.total"])
        );
        assert_eq!(
            calls_from(&result, "T.java::Shop.b"),
            set(&["T.java::Cart", "T.java::Cart.total"])
        );
        assert_eq!(
            calls_from(&result, "T.java::Shop.c"),
            set(&["total", "T.java::Cart.make"])
        );
        assert_eq!(calls_from(&result, "T.java::Shop.d"), set(&["total"]));
    }

    #[test]
    fn spring_framework_edges_match_crg() {
        let source = "import org.springframework.context.ApplicationEventPublisher;\n\
                       import org.springframework.context.event.EventListener;\n\
                       class Events {\n\
                       private final ApplicationEventPublisher publisher;\n\
                       void publish() { publisher.publishEvent(new LocalEvent()); }\n\
                       @EventListener(LocalEvent.class) void on(LocalEvent event) {}\n\
                       }\n";
        let result = extract_java(source, "E.java");
        let publishes = edges(&result, EdgeType::Other("publishes".into()));
        assert_eq!(
            publishes,
            BTreeSet::from([(
                "E.java::Events.publish".to_string(),
                "event::LocalEvent".to_string()
            )])
        );
        let handles = edges(&result, EdgeType::Other("handles".into()));
        assert!(
            handles
                .iter()
                .any(|(from, to)| from == "E.java::Events.on" && to == "event::LocalEvent")
        );
        let node = node(&result, "event::LocalEvent");
        assert_eq!(node.name.as_str(), "LocalEvent");
    }

    #[test]
    fn typed_receiver_metadata_survives_cross_file_types() {
        let source = "import com.acme.Cart;\n\
                       class Shop {\n\
                       void a(Cart cart) { cart.total(); }\n\
                       void b(com.acme.Price price) { price.amount(); }\n\
                       }\n";
        let result = extract_java(source, "T.java");
        let typed: BTreeSet<_> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .filter_map(|e| {
                let type_name = e.metadata.get("receiver_type")?;
                Some((e.to.clone(), type_name.clone()))
            })
            .collect();
        assert_eq!(
            typed,
            BTreeSet::from([
                ("total".to_string(), "Cart".to_string()),
                ("amount".to_string(), "com.acme.Price".to_string()),
            ])
        );
    }

    #[test]
    fn spring_calls_keep_their_receiver() {
        let result = extract_java(SPRING, "D.java");
        let receivers: BTreeSet<(String, String)> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .filter_map(|e| Some((e.to.clone(), e.metadata.get("receiver")?.clone())))
            .collect();
        // The declared interface answers here; the indexer's Spring pass picks the bean.
        assert!(receivers.contains(&(
            "D.java::OrderRepository.save".into(),
            "orderRepository".into()
        )));
        assert!(receivers.contains(&(
            "D.java::OrderRepository.findById".into(),
            "orderRepository".into()
        )));
        assert_eq!(
            calls_from(&result, "D.java::AppConfig.orderRepository"),
            set(&["D.java::JpaOrderRepository"])
        );
    }

    #[test]
    fn inherits_and_injects_like_crg() {
        let result = extract_java(SAMPLE, "S.java");
        assert_eq!(
            edges(&result, EdgeType::Extends),
            BTreeSet::from([
                ("S.java::CachedRepo".to_string(), "InMemoryRepo".to_string()),
                (
                    "S.java::InMemoryRepo".to_string(),
                    "UserRepository".to_string()
                ),
            ])
        );
        let result = extract_java(SPRING, "D.java");
        let injects: BTreeSet<(String, String, String, String)> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Other(super::INJECTS.into()))
            .map(|e| {
                (
                    qualified(&result, &e.from),
                    e.to.clone(),
                    e.metadata["injection_type"].clone(),
                    e.metadata["java_type"].clone(),
                )
            })
            .collect();
        let row = |from: &str, to: &str, how: &str| {
            (
                format!("D.java::{from}"),
                to.to_string(),
                how.to_string(),
                format!("com.example.shop.{to}"),
            )
        };
        assert_eq!(
            injects,
            BTreeSet::from([
                row("AuditLogger", "OrderRepository", "constructor"),
                row("NotificationService", "OrderRepository", "field"),
                row("OrderService", "NotificationService", "constructor_lombok"),
                row("OrderService", "OrderRepository", "constructor_lombok"),
            ])
        );
        assert!(edges(&result, EdgeType::Extends).contains(&(
            "D.java::JpaOrderRepository".into(),
            "OrderRepository".into()
        )));
    }

    #[test]
    fn generic_bases_and_imported_java_types() {
        let source = "package app;\nimport lib.Base;\nimport other.*;\n\
                      class A extends Base<String> implements Runnable, x.Y { }\n\
                      @Component @AllArgsConstructor class B { private Helper h; \
                      private static Helper s; }\n";
        let result = extract_java(source, "A.java");
        let extends: BTreeSet<(String, String)> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Extends)
            .map(|e| (e.to.clone(), e.metadata["java_type"].clone()))
            .collect();
        assert_eq!(
            extends,
            BTreeSet::from([
                ("Base<String>".to_string(), "Base<String>".to_string()),
                ("Runnable".to_string(), "Runnable".to_string()),
            ])
        );
        let injects: Vec<(String, String)> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Other(super::INJECTS.into()))
            .map(|e| (e.to.clone(), e.metadata["injection_type"].clone()))
            .collect();
        assert_eq!(
            injects,
            vec![("Helper".into(), "constructor_lombok_all".into())]
        );
    }

    #[test]
    fn test_annotations_make_test_nodes() {
        let source = "class CartCheck {\n\
                      @Test void totals() {}\n\
                      @org.junit.jupiter.api.Test void sums() {}\n\
                      void helper() {}\n\
                      }\n";
        let result = extract_java(source, "src/CartCheck.java");
        let kind = |name: &str| node(&result, name).node_type.clone();
        let test = Some(NodeType::Other("Test".into()));
        assert_eq!(kind("src/CartCheck.java::CartCheck.totals"), test);
        assert_eq!(kind("src/CartCheck.java::CartCheck.sums"), test);
        assert_eq!(
            kind("src/CartCheck.java::CartCheck.helper"),
            Some(NodeType::Function)
        );
    }

    #[test]
    fn base_type_names_like_crg() {
        assert_eq!(base_type_name("Cart").as_deref(), Some("Cart"));
        assert_eq!(base_type_name("Optional<Cart>").as_deref(), Some("Cart"));
        assert_eq!(base_type_name("com.acme.Cart").as_deref(), Some("Cart"));
        assert_eq!(base_type_name("Cart<T>").as_deref(), Some("Cart"));
        assert_eq!(base_type_name("List<Cart>"), None);
        assert_eq!(base_type_name("Cart[]"), None);
        assert_eq!(base_type_name("int"), None);
        assert_eq!(base_type_name("Optional"), None);
        assert_eq!(
            package_qualified_prefix("com.acme.Cart"),
            Some("com.acme.Cart")
        );
        assert_eq!(
            package_qualified_prefix("com.acme.Cart<T>"),
            Some("com.acme.Cart")
        );
        assert_eq!(package_qualified_prefix("Cart"), None);
        assert_eq!(package_qualified_prefix("a.b"), None);
    }
}
