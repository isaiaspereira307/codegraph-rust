// ABOUTME: Python AST extractor in code-review-graph's shape
// ABOUTME: Extracts functions, classes, imports, CRG call targets/receivers, dead guards and INHERITS

use super::crg_shape;
use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Node, Tree};

/// Python AST extractor in code-review-graph's shape.
///
/// Extracts:
/// - functions (`def`, `async def`) and classes, with their `decorators`
/// - imports (one `Imports` edge per statement)
/// - calls by their final name (`a.b.m()` -> `m`), leaving the innermost `def`, or the file
///   from class bodies, decorators and module code; calls in statically dead branches
///   (`if False:`, `if TYPE_CHECKING:`) are dropped
/// - receivers: `self`/`cls` and local classes name `<file>::Class.m`; imported names carry
///   `crg_py_import`; any other receiver is `crg_unbound`
/// - class bases (`identifier`/`attribute`) as `Extends` text
pub struct PythonExtractor;

impl PythonExtractor {
    pub fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        Self::extract_with_policy(tree, content, file_path, Default::default())
    }

    pub fn extract_with_policy(
        tree: &Tree,
        content: &str,
        file_path: &str,
        policy: super::ExtractionPolicy,
    ) -> ExtractionResult {
        let root = tree.root_node();
        let scope = FileScope::collect(root, content);
        let mut collector = PythonCollector::new(content, file_path, scope);
        collector.references = policy.references;
        collector.walk(root);
        ExtractionResult {
            nodes: collector.nodes,
            edges: collector.edges,
        }
    }
}

impl super::LanguageExtractor for PythonExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        PythonExtractor::extract_with_edges(tree, content, file_path)
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[
            EdgeType::Imports,
            EdgeType::Calls,
            EdgeType::Extends,
            EdgeType::References,
        ]
    }

    fn language() -> Language {
        Language::Python
    }
}

/// What the top level of a Python file binds, read before the walk (CRG
/// `_collect_file_scope` and `_python_type_checking_aliases`).
#[derive(Debug, Default, Clone)]
pub struct FileScope {
    /// Functions and classes defined at the top level.
    pub defined: HashSet<String>,
    /// Classes defined at the top level.
    pub classes: HashSet<String>,
    /// `from M import X [as Y]`: local name -> (module as written, dots included; `X`).
    pub from_imports: HashMap<String, (String, String)>,
    /// `import a.b as c`: `c` -> `a.b`; plain `import a.b`: `a` -> `a`.
    pub module_aliases: HashMap<String, String>,
    /// Unshadowed local names of `typing.TYPE_CHECKING`.
    pub type_checking: HashSet<String>,
    /// Unshadowed local names of the `typing` module (`t.TYPE_CHECKING`).
    pub typing_modules: HashSet<String>,
    /// Every class of the file: leaf name -> dotted class chains (`Outer.Inner`) using it.
    pub class_chains: HashMap<String, HashSet<String>>,
}

impl FileScope {
    pub fn collect(root: Node, content: &str) -> Self {
        let text = |n: &Node| node_text(n, content);
        let mut scope = FileScope::default();
        for child in named_children(&root) {
            let target = unwrap_decorated(child);
            match target.kind() {
                "function_definition" | "class_definition" => {
                    if let Some(name) = target.child_by_field_name("name") {
                        let name = text(&name);
                        if target.kind() == "class_definition" {
                            scope.classes.insert(name.clone());
                        }
                        scope.defined.insert(name);
                    }
                }
                "import_from_statement" => {
                    let Some(module) = target.child_by_field_name("module_name") else {
                        continue;
                    };
                    let module = text(&module);
                    for (original, local) in imported_names(&target, content) {
                        scope.from_imports.insert(local, (module.clone(), original));
                    }
                }
                "import_statement" => {
                    for name in field_children(&target, "name") {
                        if name.kind() == "dotted_name" {
                            // `import a.b` binds only `a`, to the module `a`.
                            let root = text(&name)
                                .split('.')
                                .next()
                                .unwrap_or_default()
                                .to_string();
                            scope.module_aliases.insert(root.clone(), root);
                            continue;
                        }
                        if name.kind() != "aliased_import" {
                            continue;
                        }
                        if let (Some(module), Some(alias)) = (
                            name.child_by_field_name("name"),
                            name.child_by_field_name("alias"),
                        ) {
                            scope.module_aliases.insert(text(&alias), text(&module));
                        }
                    }
                }
                _ => {}
            }
        }
        scope.collect_type_checking(root, content);
        scope.collect_class_chains(root, content, None);
        scope
    }

    /// Dotted chain of each class through its enclosing classes (functions are transparent),
    /// as crg_shape names them.
    fn collect_class_chains(&mut self, node: Node, content: &str, chain: Option<&str>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_definition"
                && let Some(name) = child.child_by_field_name("name")
            {
                let name = node_text(&name, content);
                let path = chain.map_or_else(|| name.clone(), |c| format!("{c}.{name}"));
                self.class_chains
                    .entry(name)
                    .or_default()
                    .insert(path.clone());
                self.collect_class_chains(child, content, Some(&path));
            } else {
                self.collect_class_chains(child, content, chain);
            }
        }
    }

    /// CRG `_python_type_checking_aliases`: only when the file imports `typing` or
    /// `TYPE_CHECKING`, minus every other top-level binding of the same names.
    fn collect_type_checking(&mut self, root: Node, content: &str) {
        let is_typing_from = |n: &Node| {
            n.kind() == "import_from_statement"
                && n.child_by_field_name("module_name")
                    .is_some_and(|m| node_text(&m, content) == "typing")
        };
        let statements: Vec<Node> = named_children(&root).collect();
        let has_typing_import = statements.iter().any(|s| {
            if s.kind() == "import_statement" {
                imported_names(s, content)
                    .iter()
                    .any(|(original, _)| original == "typing")
            } else if is_typing_from(s) {
                imported_names(s, content)
                    .iter()
                    .any(|(original, _)| original == "TYPE_CHECKING")
            } else {
                false
            }
        });
        if !has_typing_import {
            return;
        }
        let mut shadowed = HashSet::new();
        for statement in statements {
            if statement.kind() == "import_statement" {
                for (original, local) in imported_names(&statement, content) {
                    if original == "typing" {
                        self.typing_modules.insert(local);
                    } else {
                        shadowed.insert(local);
                    }
                }
            } else if is_typing_from(&statement) {
                for (original, local) in imported_names(&statement, content) {
                    if original == "TYPE_CHECKING" {
                        self.type_checking.insert(local);
                    } else {
                        shadowed.insert(local);
                    }
                }
            } else {
                bindings(statement, content, &mut shadowed);
            }
        }
        self.type_checking.retain(|n| !shadowed.contains(n));
        self.typing_modules.retain(|n| !shadowed.contains(n));
    }
}

/// One caller frame: a `def` (`Some`) or a class body (`None`, whose calls leave the file).
struct Frame {
    caller: Option<NodeId>,
    /// `def`s directly in this function's body: name -> qualified name.
    local_defs: HashMap<String, String>,
    /// `TYPE_CHECKING` names and `typing` modules still unshadowed in this scope.
    type_checking: HashSet<String>,
    typing_modules: HashSet<String>,
}

struct PythonCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    scope: FileScope,
    frames: Vec<Frame>,
    /// Scopes of the classes around the walk position, innermost last: the leaf name, or
    /// the full chain for a nested class whose leaf another class of the file shares.
    classes: Vec<String>,
    /// Dotted chains (`Outer.Inner`) of the classes around the walk position.
    class_chains: Vec<String>,
    /// Depth of statically dead branches around the walk position.
    dead: usize,
    /// Whether value `References` edges are emitted (`ExtractionPolicy.references`).
    references: bool,
}

impl<'a> PythonCollector<'a> {
    fn new(content: &'a str, file_path: &'a str, scope: FileScope) -> Self {
        Self {
            content,
            file_path,
            nodes: Vec::new(),
            edges: Vec::new(),
            scope,
            frames: Vec::new(),
            classes: Vec::new(),
            class_chains: Vec::new(),
            dead: 0,
            references: true,
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => return self.function(node),
            "class_definition" => return self.class(node),
            "if_statement" => return self.if_statement(node),
            "import_statement" | "import_from_statement" => self.import(&node),
            "call" => self.call(&node),
            "argument_list" => self.argument_references(&node),
            "list" => self.list_references(&node),
            "pair" => self.pair_reference(&node),
            "assignment" | "augmented_assignment" => self.assignment_reference(&node),
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

    fn walk_dead(&mut self, node: Node, dead: bool) {
        self.dead += usize::from(dead);
        self.walk(node);
        self.dead -= usize::from(dead);
    }

    /// CRG `_PythonUnreachableCallVisitor.visit_If`: the body of a statically false
    /// condition is dead, and so is everything after a statically true one (`elif`/`else`).
    fn if_statement(&mut self, node: Node) {
        let truth = node
            .child_by_field_name("condition")
            .and_then(|c| self.static_truth(&c));
        let consequence = node.child_by_field_name("consequence").map(|c| c.id());
        let mut rest_dead = truth == Some(true);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if Some(child.id()) == consequence {
                self.walk_dead(child, truth == Some(false));
                continue;
            }
            match child.kind() {
                "elif_clause" if !rest_dead => {
                    let truth = child
                        .child_by_field_name("condition")
                        .and_then(|c| self.static_truth(&c));
                    let consequence = child.child_by_field_name("consequence").map(|c| c.id());
                    let mut inner = child.walk();
                    for part in child.children(&mut inner) {
                        let dead = Some(part.id()) == consequence && truth == Some(false);
                        self.walk_dead(part, dead);
                    }
                    rest_dead = truth == Some(true);
                }
                "elif_clause" | "else_clause" => self.walk_dead(child, rest_dead),
                _ => self.walk(child),
            }
        }
    }

    /// CRG `_python_static_truth`: `True`/`False`, integers, `not`, `and`/`or`, parentheses
    /// and unshadowed `TYPE_CHECKING` (always false); anything else is unknown.
    fn static_truth(&self, node: &Node) -> Option<bool> {
        match node.kind() {
            "true" => Some(true),
            "false" => Some(false),
            "integer" => integer_truth(&self.text(node)),
            "identifier" => self
                .type_checking()
                .contains(&self.text(node))
                .then_some(false),
            "attribute" => {
                let object = node.child_by_field_name("object")?;
                let attribute = node.child_by_field_name("attribute")?;
                (object.kind() == "identifier"
                    && self.typing_modules().contains(&self.text(&object))
                    && self.text(&attribute) == "TYPE_CHECKING")
                    .then_some(false)
            }
            "not_operator" => node
                .child_by_field_name("argument")
                .and_then(|a| self.static_truth(&a))
                .map(|v| !v),
            "boolean_operator" => {
                let left = node
                    .child_by_field_name("left")
                    .and_then(|n| self.static_truth(&n));
                let right = node
                    .child_by_field_name("right")
                    .and_then(|n| self.static_truth(&n));
                let operator = node.child_by_field_name("operator")?;
                match operator.kind() {
                    "and" if left == Some(false) || right == Some(false) => Some(false),
                    "and" if left == Some(true) && right == Some(true) => Some(true),
                    "or" if left == Some(true) || right == Some(true) => Some(true),
                    "or" if left == Some(false) && right == Some(false) => Some(false),
                    _ => None,
                }
            }
            "parenthesized_expression" => named_children(node)
                .find(|c| c.kind() != "comment")
                .and_then(|inner| self.static_truth(&inner)),
            _ => None,
        }
    }

    fn type_checking(&self) -> &HashSet<String> {
        self.frames
            .last()
            .map_or(&self.scope.type_checking, |f| &f.type_checking)
    }

    fn typing_modules(&self) -> &HashSet<String> {
        self.frames
            .last()
            .map_or(&self.scope.typing_modules, |f| &f.typing_modules)
    }

    /// A new scope's `TYPE_CHECKING` names and `typing` modules, minus what it rebinds.
    fn scoped_type_checking(&self, bound: &HashSet<String>) -> (HashSet<String>, HashSet<String>) {
        let keep = |set: &HashSet<String>| -> HashSet<String> {
            set.iter()
                .filter(|n| !bound.contains(*n))
                .cloned()
                .collect()
        };
        (keep(self.type_checking()), keep(self.typing_modules()))
    }

    fn function(&mut self, node: Node) {
        let Some(name) = self.field_text(&node, "name") else {
            return self.walk_children(node);
        };
        let mut code = CodeNode::new(
            name,
            Some(NodeType::Function),
            Some(Language::Python),
            self.location(&node),
        )
        .with_content(self.text(&node))
        .with_complexity(crate::complexity::calculate_cyclomatic_complexity(
            &node,
            self.content,
        ));
        code.span = Some(self.span_for(&node));
        self.record_decorators(&node, &mut code);
        let id = code.id;
        self.nodes.push(code);

        // A `def` nested in this body hangs off the innermost class around it (crg_shape).
        let prefix = match self.classes.last() {
            Some(class) => format!("{}::{class}.", self.file_path),
            None => format!("{}::", self.file_path),
        };
        let mut local_defs = HashMap::new();
        let mut bound = HashSet::new();
        if let Some(body) = node.child_by_field_name("body") {
            for inner in nested_defs(body) {
                if let Some(inner_name) = self.field_text(&inner, "name") {
                    local_defs.insert(inner_name.clone(), format!("{prefix}{inner_name}"));
                }
            }
            for statement in named_children(&body) {
                bindings(statement, self.content, &mut bound);
            }
        }
        if let Some(parameters) = node.child_by_field_name("parameters") {
            parameter_names(&parameters, self.content, &mut bound);
        }
        let (type_checking, typing_modules) = self.scoped_type_checking(&bound);
        self.frames.push(Frame {
            caller: Some(id),
            local_defs,
            type_checking,
            typing_modules,
        });
        self.walk_children(node);
        self.frames.pop();
    }

    fn class(&mut self, node: Node) {
        let Some(name) = self.field_text(&node, "name") else {
            return self.walk_children(node);
        };
        let mut code = CodeNode::new(
            name.clone(),
            Some(NodeType::Class),
            Some(Language::Python),
            self.location(&node),
        )
        .with_content(self.text(&node));
        code.span = Some(self.span_for(&node));
        self.record_decorators(&node, &mut code);

        // INHERITS: plain and dotted bases, as written; keywords and subscripts are not bases.
        if let Some(bases) = node.child_by_field_name("superclasses") {
            for base in
                named_children(&bases).filter(|b| matches!(b.kind(), "identifier" | "attribute"))
            {
                self.edges.push(EdgeRelationship {
                    from: code.id,
                    to: self.text(&base),
                    edge_type: EdgeType::Extends,
                    metadata: HashMap::from([(
                        "source_file".to_string(),
                        self.file_path.to_string(),
                    )]),
                    span: Some(self.span_for(&base)),
                });
            }
        }
        self.nodes.push(code);

        let mut bound = HashSet::new();
        if let Some(body) = node.child_by_field_name("body") {
            for statement in named_children(&body) {
                bindings(statement, self.content, &mut bound);
            }
        }
        let (type_checking, typing_modules) = self.scoped_type_checking(&bound);
        self.frames.push(Frame {
            caller: None,
            local_defs: HashMap::new(),
            type_checking,
            typing_modules,
        });
        let chain = match self.class_chains.last() {
            Some(outer) => format!("{outer}.{name}"),
            None => name.clone(),
        };
        let ambiguous = self
            .scope
            .class_chains
            .get(&name)
            .is_some_and(|chains| chains.len() > 1);
        let class_scope = if ambiguous && !self.class_chains.is_empty() {
            chain.clone()
        } else {
            name
        };
        self.classes.push(class_scope);
        self.class_chains.push(chain);
        self.walk_children(node);
        self.class_chains.pop();
        self.classes.pop();
        self.frames.pop();
    }

    /// `decorators`: each decorator of the wrapping `decorated_definition`, without `@`.
    fn record_decorators(&self, node: &Node, code: &mut CodeNode) {
        let Some(parent) = node.parent().filter(|p| p.kind() == "decorated_definition") else {
            return;
        };
        let decorators: Vec<String> = named_children(&parent)
            .filter(|c| c.kind() == "decorator")
            .map(|d| {
                self.text(&d)
                    .trim()
                    .trim_start_matches('@')
                    .trim()
                    .to_string()
            })
            .collect();
        if !decorators.is_empty() {
            code.metadata
                .attributes
                .insert(crg_shape::DECORATORS_ATTR.into(), decorators.join(","));
        }
    }

    /// CRG `_extract_import`: `import a.b, c as d` imports each module; `from M import x, y`
    /// imports `M` plus the candidates `M.x`, `M.y` (marked `PY_SUBMODULE_ATTR`), which the
    /// indexer keeps only when they name a file; `from M import *` imports `M`.
    fn import(&mut self, node: &Node) {
        // (target, is a submodule candidate)
        let targets: Vec<(String, bool)> = if node.kind() == "import_statement" {
            imported_names(node, self.content)
                .into_iter()
                .map(|(module, _)| (module, false))
                .collect()
        } else {
            let Some(module) = node.child_by_field_name("module_name") else {
                return;
            };
            let module = self.text(&module);
            let prefix = if module.ends_with('.') {
                module.clone()
            } else {
                format!("{module}.")
            };
            std::iter::once((module, false))
                .chain(
                    imported_names(node, self.content)
                        .into_iter()
                        .map(|(name, _)| (format!("{prefix}{name}"), true)),
                )
                .collect()
        };
        let Some((name, _)) = targets.first() else {
            return;
        };
        let mut code = CodeNode::new(
            name.clone(),
            Some(NodeType::Import),
            Some(Language::Python),
            self.location(node),
        )
        .with_content(self.text(node));
        code.span = Some(self.span_for(node));
        for (target, submodule) in targets {
            let mut metadata = HashMap::from([
                ("import_type".to_string(), "python_import".to_string()),
                ("source_file".to_string(), self.file_path.to_string()),
            ]);
            if submodule {
                metadata.insert(crg_shape::PY_SUBMODULE_ATTR.to_string(), "true".to_string());
            }
            self.edges.push(EdgeRelationship {
                from: code.id,
                to: target,
                edge_type: EdgeType::Imports,
                metadata,
                span: Some(self.span_for(node)),
            });
        }
        self.nodes.push(code);
    }

    fn call(&mut self, node: &Node) {
        if self.dead > 0 {
            return;
        }
        let Some(callee) = node.child_by_field_name("function") else {
            return;
        };
        let mut metadata = HashMap::from([
            ("call_type".to_string(), "python_call".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        let target = match callee.kind() {
            "identifier" => self.bare_target(&self.text(&callee), &mut metadata),
            "attribute" => {
                let (Some(object), Some(method)) = (
                    callee.child_by_field_name("object"),
                    callee.child_by_field_name("attribute"),
                ) else {
                    return;
                };
                metadata.insert("receiver".to_string(), self.text(&object));
                self.receiver_target(&object, self.text(&method), &mut metadata)
            }
            // Calls of calls, subscripts and parenthesized callees: no edge.
            _ => return,
        };

        self.push_edge(EdgeType::Calls, target, metadata, node);
    }

    /// An edge from the innermost `def`; a class body (or no frame) is the file.
    fn push_edge(
        &mut self,
        edge_type: EdgeType,
        to: String,
        mut metadata: HashMap<String, String>,
        node: &Node,
    ) {
        let from = match self.frames.last().and_then(|f| f.caller) {
            Some(id) => id,
            None => {
                metadata.insert(crg_shape::FILE_SCOPE_ATTR.to_string(), "true".to_string());
                NodeId::nil()
            }
        };
        self.edges.push(EdgeRelationship {
            from,
            to,
            edge_type,
            metadata,
            span: Some(self.span_for(node)),
        });
    }

    /// CRG `_emit_reference_if_known`: a plain identifier naming a top-level def/class or a
    /// `from` import references it, with the target of a receiver-less call.
    fn reference(&mut self, node: &Node) {
        if !self.references || node.kind() != "identifier" {
            return;
        }
        let name = self.text(node);
        if matches!(
            name.as_str(),
            "self" | "cls" | "super" | "None" | "True" | "False"
        ) || name.chars().count() <= 1
            || is_upper(&name)
            || !(self.scope.defined.contains(&name) || self.scope.from_imports.contains_key(&name))
        {
            return;
        }
        let mut metadata = HashMap::from([
            ("kind".to_string(), "value".to_string()),
            ("source_file".to_string(), self.file_path.to_string()),
        ]);
        let target = self.bare_target(&name, &mut metadata);
        self.push_edge(EdgeType::References, target, metadata, node);
    }

    /// Call arguments and class bases: identifiers and `key=identifier` values.
    fn argument_references(&mut self, node: &Node) {
        for child in named_children(node) {
            if child.kind() == "keyword_argument" {
                if let Some(value) = child.child_by_field_name("value") {
                    self.reference(&value);
                }
            } else {
                self.reference(&child);
            }
        }
    }

    fn list_references(&mut self, node: &Node) {
        for child in named_children(node) {
            self.reference(&child);
        }
    }

    fn pair_reference(&mut self, node: &Node) {
        if let Some(value) = node.child_by_field_name("value") {
            self.reference(&value);
        }
    }

    /// `obj.attr = f`, `obj[k] = f` and `obj.attr += f`; plain `h = f` is not a reference.
    fn assignment_reference(&mut self, node: &Node) {
        let is_member = node
            .child_by_field_name("left")
            .is_some_and(|l| matches!(l.kind(), "attribute" | "subscript"));
        if let (true, Some(right)) = (is_member, node.child_by_field_name("right")) {
            self.reference(&right);
        }
    }

    /// `name()`: a `def` of an enclosing function, then a top-level def/class, then a name
    /// imported with `from` (bare name plus `crg_py_import`); otherwise the bare name.
    fn bare_target(&self, name: &str, metadata: &mut HashMap<String, String>) -> String {
        if let Some(local) = self
            .frames
            .iter()
            .rev()
            .filter(|f| f.caller.is_some())
            .find_map(|f| f.local_defs.get(name))
        {
            return local.clone();
        }
        if self.scope.defined.contains(name) {
            return format!("{}::{name}", self.file_path);
        }
        if let Some((module, original)) = self.scope.from_imports.get(name) {
            metadata.insert(
                crg_shape::PY_IMPORT_ATTR.to_string(),
                format!("{module}|{original}"),
            );
        }
        name.to_string()
    }

    /// `object.method()` by what `object` is: `self`/`cls` in a class and local classes name
    /// `<file>::Class.method`; names imported with `from` or `import a.b as c` carry
    /// `crg_py_import`; `X()` stands for a local class or `from` import `X`; anything else
    /// is `crg_unbound`.
    fn receiver_target(
        &self,
        object: &Node,
        method: String,
        metadata: &mut HashMap<String, String>,
    ) -> String {
        let receiver = match object.kind() {
            "identifier" => {
                let name = self.text(object);
                if matches!(name.as_str(), "self" | "cls")
                    && let Some(class) = self.classes.last()
                {
                    return format!("{}::{class}.{method}", self.file_path);
                }
                Some(name)
            }
            "call" => object
                .child_by_field_name("function")
                .filter(|f| f.kind() == "identifier")
                .map(|f| self.text(&f))
                .filter(|x| {
                    self.scope.classes.contains(x) || self.scope.from_imports.contains_key(x)
                }),
            _ => None,
        };
        let Some(receiver) = receiver else {
            metadata.insert(crg_shape::UNBOUND_ATTR.to_string(), "true".to_string());
            return method;
        };
        if self.scope.classes.contains(&receiver) {
            return format!("{}::{receiver}.{method}", self.file_path);
        }
        if let Some((module, original)) = self.scope.from_imports.get(&receiver) {
            metadata.insert(
                crg_shape::PY_IMPORT_ATTR.to_string(),
                format!("{module}|{original}"),
            );
        } else if let Some(module) = self.scope.module_aliases.get(&receiver) {
            metadata.insert(crg_shape::PY_IMPORT_ATTR.to_string(), format!("{module}|"));
        } else {
            metadata.insert(crg_shape::UNBOUND_ATTR.to_string(), "true".to_string());
        }
        method
    }

    fn span_for(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
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

    fn text(&self, node: &Node) -> String {
        node_text(node, self.content)
    }

    fn field_text(&self, node: &Node, field: &str) -> Option<String> {
        node.child_by_field_name(field)
            .map(|n| self.text(&n))
            .filter(|text| !text.is_empty())
    }
}

/// Python `str.isupper`: has a cased character and no lowercase one.
fn is_upper(name: &str) -> bool {
    name.chars().any(char::is_uppercase) && !name.chars().any(char::is_lowercase)
}

/// Truth of a Python integer literal (`0`, `0x0`, `1_000`); `1j` is complex: unknown.
fn integer_truth(literal: &str) -> Option<bool> {
    let literal = literal.to_ascii_lowercase().replace('_', "");
    if literal.ends_with('j') {
        return None;
    }
    let digits = ["0x", "0o", "0b"]
        .iter()
        .find_map(|prefix| literal.strip_prefix(prefix))
        .unwrap_or(&literal);
    Some(digits.chars().any(|c| c != '0'))
}

/// The definition inside a `decorated_definition`, or the node itself.
fn unwrap_decorated(node: Node) -> Node {
    if node.kind() == "decorated_definition" {
        node.child_by_field_name("definition").unwrap_or(node)
    } else {
        node
    }
}

/// `(original, local)` names an import statement binds: `import a.b` -> `(a.b, a)`,
/// `import a.b as c` -> `(a.b, c)`, `from M import x as y` -> `(x, y)`; `*` binds nothing.
fn imported_names(node: &Node, content: &str) -> Vec<(String, String)> {
    let from = node.kind() == "import_from_statement";
    field_children(node, "name")
        .into_iter()
        .filter_map(|name| match name.kind() {
            "dotted_name" => {
                let original = node_text(&name, content);
                let local = if from {
                    original.clone()
                } else {
                    original.split('.').next().unwrap_or_default().to_string()
                };
                Some((original, local))
            }
            "aliased_import" => Some((
                node_text(&name.child_by_field_name("name")?, content),
                node_text(&name.child_by_field_name("alias")?, content),
            )),
            _ => None,
        })
        .collect()
}

/// CRG `_PythonScopeBindingVisitor`: names a statement binds in its own scope (assignment,
/// loop, comprehension, `with` and walrus targets, `def`/`class` names, imports, `global`,
/// `del`), without entering nested `def`s, classes or lambdas.
fn bindings(node: Node, content: &str, out: &mut HashSet<String>) {
    match node.kind() {
        "function_definition" | "class_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                out.insert(node_text(&name, content));
            }
            return;
        }
        "lambda" => return,
        "import_statement" | "import_from_statement" => {
            out.extend(imported_names(&node, content).into_iter().map(|(_, l)| l));
            return;
        }
        "global_statement" => {
            out.extend(
                named_children(&node)
                    .filter(|c| c.kind() == "identifier")
                    .map(|c| node_text(&c, content)),
            );
            return;
        }
        "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
            if let Some(left) = node.child_by_field_name("left") {
                targets(left, content, out);
            }
        }
        "named_expression" => {
            if let Some(name) = node.child_by_field_name("name") {
                out.insert(node_text(&name, content));
            }
        }
        "delete_statement" => {
            for target in named_children(&node) {
                targets(target, content, out);
            }
        }
        "as_pattern" if node.parent().is_some_and(|p| p.kind() == "with_item") => {
            if let Some(alias) = node.child_by_field_name("alias") {
                targets(alias, content, out);
            }
        }
        _ => {}
    }
    for child in named_children(&node) {
        bindings(child, content, out);
    }
}

/// Plain names an assignment-like target binds (`a`, `a, (b, *c)`); attributes and
/// subscripts bind none.
fn targets(node: Node, content: &str, out: &mut HashSet<String>) {
    match node.kind() {
        "identifier" => {
            out.insert(node_text(&node, content));
        }
        "pattern_list"
        | "tuple_pattern"
        | "list_pattern"
        | "tuple"
        | "list"
        | "expression_list"
        | "list_splat_pattern"
        | "parenthesized_expression"
        | "as_pattern_target" => {
            for child in named_children(&node) {
                targets(child, content, out);
            }
        }
        _ => {}
    }
}

/// Parameter names of a `def` (`a`, `a=1`, `a: int`, `*args`, `**kw`).
fn parameter_names(parameters: &Node, content: &str, out: &mut HashSet<String>) {
    for parameter in named_children(parameters) {
        let mut name = match parameter.kind() {
            "default_parameter" | "typed_default_parameter" => {
                parameter.child_by_field_name("name")
            }
            "typed_parameter" => named_children(&parameter).next(),
            _ => Some(parameter),
        };
        if let Some(splat) =
            name.filter(|n| matches!(n.kind(), "list_splat_pattern" | "dictionary_splat_pattern"))
        {
            name = named_children(&splat).next();
        }
        if let Some(name) = name.filter(|n| n.kind() == "identifier") {
            out.insert(node_text(&name, content));
        }
    }
}

/// `def`s in a function body, through compound statements but not into nested functions,
/// classes or lambdas.
fn nested_defs(body: Node) -> Vec<Node> {
    let mut defs = Vec::new();
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        for child in named_children(&node) {
            match child.kind() {
                "function_definition" => defs.push(child),
                "class_definition" | "lambda" => {}
                _ => stack.push(child),
            }
        }
    }
    defs
}

fn node_text(node: &Node, content: &str) -> String {
    node.utf8_text(content.as_bytes()).unwrap_or("").to_string()
}

fn field_children<'t>(node: &Node<'t>, field: &str) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children_by_field_name(field, &mut cursor).collect()
}

fn named_children<'t>(node: &Node<'t>) -> impl Iterator<Item = Node<'t>> {
    let node = *node;
    (0..node.named_child_count() as u32).filter_map(move |i| node.named_child(i))
}

#[cfg(test)]
mod tests {
    use crate::languages::crg_shape;
    use codegraph_core::{EdgeType, ExtractionResult, Language};
    use std::collections::BTreeSet;

    fn extract_py(source: &str, path: &str) -> ExtractionResult {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        crate::languages::extract_for_language_at(
            &Language::Python,
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

    /// `attr` of the first edge of `kind` whose target is `to`.
    fn edge_attr(
        result: &ExtractionResult,
        kind: EdgeType,
        to: &str,
        attr: &str,
    ) -> Option<String> {
        result
            .edges
            .iter()
            .find(|e| e.edge_type == kind && e.to == to)
            .and_then(|e| e.metadata.get(attr).cloned())
    }

    fn calls_from(result: &ExtractionResult, source: &str) -> BTreeSet<String> {
        edges(result, EdgeType::Calls)
            .into_iter()
            .filter(|(from, _)| from == source)
            .map(|(_, to)| to)
            .collect()
    }

    fn set(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        items
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    fn names(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn call_names_and_callers_follow_crg() {
        let r = extract_py(
            "import json as js\nCONST = helper(1)\n\nclass Base:\n    x = make()\n    def step(self):\n        return 1\n\n\
             class Child(Base):\n    @staticmethod\n    @deco.wrap(1)\n    def run(self):\n        self.step()\n        Base.step(self)\n        js.loads('x')\n        self.repo.save()\n        super().__init__()\n        get()()\n        obj['k'].m()\n        f = lambda: helper(2)\n        return [v.upper() for v in xs]\n\n\
             def outer():\n    def inner():\n        pass\n    return inner()\n\nif __name__ == '__main__':\n    main()\n",
            "m.py",
        );
        assert_eq!(
            edges(&r, EdgeType::Calls),
            set(&[
                ("m.py", "helper"),
                ("m.py", "make"),
                ("m.py", "wrap"),
                ("m.py", "main"),
                ("m.py::Child.run", "m.py::Child.step"),
                ("m.py::Child.run", "m.py::Base.step"),
                ("m.py::Child.run", "loads"),
                ("m.py::Child.run", "save"),
                ("m.py::Child.run", "super"),
                ("m.py::Child.run", "__init__"),
                ("m.py::Child.run", "get"),
                ("m.py::Child.run", "m"),
                ("m.py::Child.run", "helper"),
                ("m.py::Child.run", "upper"),
                ("m.py::outer", "m.py::inner"),
            ])
        );
        assert!(edge_attr(&r, EdgeType::Calls, "save", crg_shape::UNBOUND_ATTR).is_some());
        // `import json as js` binds a module (plan: `import a.b as c` -> "a.b|"); the indexer
        // finds no file for `json`, so the call stays unbound there, as in CRG.
        assert_eq!(
            edge_attr(&r, EdgeType::Calls, "loads", crg_shape::PY_IMPORT_ATTR).as_deref(),
            Some("json|")
        );
        assert_eq!(
            edge_attr(&r, EdgeType::Calls, "loads", "receiver").as_deref(),
            Some("js")
        );
        assert!(edge_attr(&r, EdgeType::Calls, "helper", "receiver").is_none());
    }

    #[test]
    fn imported_receivers_carry_the_import() {
        let r = extract_py(
            "from pkg import b\nfrom pkg.b import Thing, helper as hh\nimport pkg.b as bm\n\n\
             def f():\n    b.helper()\n    bm.helper()\n    hh()\n    Thing.go()\n    Thing().go()\n    b.Thing.go()\n",
            "pkg/a.py",
        );
        // One row per call (a multiset, receiver text included), so `Thing.go()` and
        // `Thing().go()` are asserted separately.
        let mut calls: Vec<(String, String, bool, String)> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .map(|e| {
                (
                    e.to.clone(),
                    e.metadata
                        .get(crg_shape::PY_IMPORT_ATTR)
                        .cloned()
                        .unwrap_or_default(),
                    e.metadata.contains_key(crg_shape::UNBOUND_ATTR),
                    e.metadata.get("receiver").cloned().unwrap_or_default(),
                )
            })
            .collect();
        calls.sort();
        let mut expected: Vec<(String, String, bool, String)> = [
            ("helper", "pkg|b", false, "b"),
            ("helper", "pkg.b|", false, "bm"),
            ("hh", "pkg.b|helper", false, ""),
            ("go", "pkg.b|Thing", false, "Thing"),
            ("go", "pkg.b|Thing", false, "Thing()"),
            ("Thing", "pkg.b|Thing", false, ""),
            ("go", "", true, "b.Thing"),
        ]
        .iter()
        .map(|(a, b, c, d)| (a.to_string(), b.to_string(), *c, d.to_string()))
        .collect();
        expected.sort();
        assert_eq!(calls, expected);
    }

    /// CRG `_python_plain_import_bindings`: `import a.b` binds `a` to module `a`, so
    /// `a.f()` names that module (the indexer binds it only when `a` is a file).
    #[test]
    fn plain_imports_bind_their_first_segment() {
        let r = extract_py(
            "import mod2\nimport nsp.inner\n\ndef f():\n    mod2.f()\n    mod2.Thing()\n    nsp.g()\n    inner.h()\n",
            "plain.py",
        );
        let calls: Vec<(String, Option<String>, bool)> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Calls)
            .map(|e| {
                (
                    e.to.clone(),
                    e.metadata.get(crg_shape::PY_IMPORT_ATTR).cloned(),
                    e.metadata.contains_key(crg_shape::UNBOUND_ATTR),
                )
            })
            .collect();
        assert_eq!(
            calls,
            vec![
                ("f".to_string(), Some("mod2|".to_string()), false),
                ("Thing".to_string(), Some("mod2|".to_string()), false),
                ("g".to_string(), Some("nsp|".to_string()), false),
                ("h".to_string(), None, true),
            ]
        );
    }

    #[test]
    fn relative_imports_carry_the_dotted_module() {
        let r = extract_py(
            "from . import x\nfrom ..p import y\n\ndef f():\n    x.a()\n    y()\n",
            "pkg/m.py",
        );
        assert_eq!(
            edge_attr(&r, EdgeType::Calls, "a", crg_shape::PY_IMPORT_ATTR).as_deref(),
            Some(".|x")
        );
        assert_eq!(
            edge_attr(&r, EdgeType::Calls, "y", crg_shape::PY_IMPORT_ATTR).as_deref(),
            Some("..p|y")
        );
    }

    #[test]
    fn local_class_instance_receiver_names_the_method() {
        let r = extract_py(
            "class X:\n    def m(self):\n        pass\n\ndef f():\n    X().m()\n",
            "m.py",
        );
        // `X()` is itself a call to the class.
        assert_eq!(calls_from(&r, "m.py::f"), names(&["m.py::X", "m.py::X.m"]));
        assert!(edge_attr(&r, EdgeType::Calls, "m.py::X.m", crg_shape::UNBOUND_ATTR).is_none());
    }

    const REFERENCE_SOURCE: &str = "from x import imp\n\nclass Base:\n    pass\n\nclass C(Base, metaclass=Base):\n    pass\n\n\
         def work():\n    pass\n\ndef use(items):\n    sorted(items, key=work)\n    run(work, cb=imp)\n    l = [work]\n    d = {'k': work}\n    obj.cb = work\n    h = work\n    t = (work,)\n    g(self, None, X, w2)\n    return work\n";

    #[test]
    fn value_references_follow_crg() {
        let r = extract_py(REFERENCE_SOURCE, "m.py");
        let mut refs: Vec<(String, String)> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::References)
            .map(|e| (qualified(&r, &e.from), e.to.clone()))
            .collect();
        refs.sort();
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        // Base appears twice in the bases list (positional and `metaclass=`), origin File;
        // `use` references work in (a one-letter name is filtered out) key=, an argument, a list, a dict value and `obj.cb = work`.
        assert_eq!(
            refs,
            vec![
                pair("m.py", "m.py::Base"),
                pair("m.py", "m.py::Base"),
                pair("m.py::use", "imp"),
                pair("m.py::use", "m.py::work"),
                pair("m.py::use", "m.py::work"),
                pair("m.py::use", "m.py::work"),
                pair("m.py::use", "m.py::work"),
                pair("m.py::use", "m.py::work"),
            ]
        );
        assert_eq!(
            edge_attr(&r, EdgeType::References, "imp", crg_shape::PY_IMPORT_ATTR).as_deref(),
            Some("x|imp")
        );
        assert!(edge_attr(&r, EdgeType::References, "imp", "receiver").is_none());
    }

    #[test]
    fn augmented_member_assignments_reference_their_value() {
        let r = extract_py(
            "def work():\n    pass\n\ndef use(obj, n):\n    obj.x += work\n    obj['k'] |= work\n    n += work\n",
            "m.py",
        );
        let refs: Vec<_> = r
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::References)
            .map(|e| (qualified(&r, &e.from), e.to.clone()))
            .collect();
        let pair = ("m.py::use".to_string(), "m.py::work".to_string());
        assert_eq!(refs, vec![pair.clone(), pair]);
    }

    #[test]
    fn references_policy_off_drops_them() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(REFERENCE_SOURCE, None).unwrap();
        let r = crate::languages::extract_for_language_at(
            &Language::Python,
            &tree,
            REFERENCE_SOURCE,
            "m.py",
            "m.py",
            crate::languages::ExtractionPolicy {
                references: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(edges(&r, EdgeType::References).is_empty());
        assert!(!edges(&r, EdgeType::Calls).is_empty());
    }

    #[test]
    fn static_dead_guards_drop_calls() {
        let r = extract_py(
            "from typing import TYPE_CHECKING\n\ndef g():\n    if False:\n        a()\n    if not True:\n        b()\n    if TYPE_CHECKING:\n        c()\n    if 0:\n        d()\n    else:\n        e()\n    if True:\n        f()\n    else:\n        h()\n    while False:\n        i()\n    if x:\n        j()\n",
            "m.py",
        );
        assert_eq!(calls_from(&r, "m.py::g"), names(&["e", "f", "i", "j"]));
    }

    #[test]
    fn dead_guards_follow_elif_chains_and_typing_aliases() {
        let r = extract_py(
            "import typing as t\nfrom typing import TYPE_CHECKING as TC\n\ndef g():\n    if x:\n        a()\n    elif False:\n        b()\n    elif True:\n        c()\n    elif y:\n        d()\n    else:\n        e()\n    if t.TYPE_CHECKING or False:\n        f()\n    if (TC and x):\n        h()\n    if None:\n        i()\n    if '':\n        j()\n",
            "m.py",
        );
        assert_eq!(calls_from(&r, "m.py::g"), names(&["a", "c", "i", "j"]));
    }

    #[test]
    fn type_checking_assigned_at_top_is_live() {
        let r = extract_py(
            "from typing import TYPE_CHECKING\nTYPE_CHECKING = True\n\ndef g():\n    if TYPE_CHECKING:\n        a()\n",
            "m.py",
        );
        assert_eq!(calls_from(&r, "m.py::g"), names(&["a"]));
    }

    #[test]
    fn type_checking_rebound_in_a_function_is_live_there() {
        let r = extract_py(
            "from typing import TYPE_CHECKING\n\ndef g(TYPE_CHECKING):\n    if TYPE_CHECKING:\n        a()\n\ndef h():\n    if TYPE_CHECKING:\n        b()\n    if 0x0 or 0:\n        c()\n",
            "m.py",
        );
        assert_eq!(calls_from(&r, "m.py::g"), names(&["a"]));
        assert!(calls_from(&r, "m.py::h").is_empty());
    }

    #[test]
    fn nested_def_decorators_leave_the_outer_function() {
        let r = extract_py(
            "def outer():\n    @wrap()\n    def inner():\n        pass\n\nclass C:\n    def m(self, x=dflt()):\n        pass\n",
            "m.py",
        );
        assert_eq!(
            edges(&r, EdgeType::Calls),
            set(&[("m.py::outer", "wrap"), ("m.py::C.m", "dflt")])
        );
    }

    #[test]
    fn nested_method_calls_and_cls() {
        let r = extract_py(
            "class K:\n    @classmethod\n    def make(cls):\n        return cls.build()\n    def build(self):\n        def nested():\n            return self.make()\n        return nested()\n",
            "m.py",
        );
        assert_eq!(
            edges(&r, EdgeType::Calls),
            set(&[
                ("m.py::K.make", "m.py::K.build"),
                ("m.py::K.nested", "m.py::K.make"),
                ("m.py::K.build", "m.py::K.nested"),
            ])
        );
    }

    /// Inside a nested class `self.m()` and nested defs use the leaf scope (`Inner.m`),
    /// unless another class of the file has the same leaf: then the full chain, as crg_shape.
    #[test]
    fn ambiguous_nested_class_leaf_uses_the_full_chain() {
        let r = extract_py(
            "class A:\n    class Inner:\n        def m(self):\n            return self.n()\n        def n(self):\n            def deep():\n                return self.m()\n            return deep()\n\
             class B:\n    class Inner:\n        def m(self):\n            return self.m()\n\
             class C:\n    class Solo:\n        def s(self):\n            return self.s()\n",
            "m.py",
        );
        assert_eq!(
            edges(&r, EdgeType::Calls),
            set(&[
                ("m.py::A.Inner.m", "m.py::A.Inner.n"),
                ("m.py::A.Inner.deep", "m.py::A.Inner.m"),
                ("m.py::A.Inner.n", "m.py::A.Inner.deep"),
                ("m.py::B.Inner.m", "m.py::B.Inner.m"),
                ("m.py::Solo.s", "m.py::Solo.s"),
            ])
        );
    }

    #[test]
    fn bases_are_inherits_text_and_decorators_recorded() {
        let r = extract_py(
            "import unittest\nclass T(Base, unittest.TestCase, Generic[X], metaclass=M):\n    pass\n\n@pytest.mark.parametrize('x', [1])\n@dec\ndef p():\n    pass\n",
            "m.py",
        );
        assert_eq!(
            edges(&r, EdgeType::Extends),
            set(&[("m.py::T", "Base"), ("m.py::T", "unittest.TestCase")])
        );
        let p = r.nodes.iter().find(|n| n.name.as_str() == "p").unwrap();
        assert_eq!(
            p.metadata.attributes[crg_shape::DECORATORS_ATTR],
            "pytest.mark.parametrize('x', [1]),dec"
        );
        assert!(edges(&r, EdgeType::References).is_empty());
    }

    #[test]
    fn imports_still_leave_the_file() {
        let r = extract_py(
            "import os\nfrom pkg import b\n\ndef f():\n    pass\n",
            "m.py",
        );
        assert_eq!(
            edges(&r, EdgeType::Imports),
            set(&[("m.py", "os"), ("m.py", "pkg"), ("m.py", "pkg.b")])
        );
    }

    #[test]
    fn imports_name_every_module_and_mark_submodule_candidates() {
        let r = extract_py(
            "import json as js, os\nimport a.b\nfrom app import util, x as y\nfrom . import z\n\
             from .. import w\nfrom .m import (p, q)\nfrom star import *\n",
            "m.py",
        );
        assert_eq!(
            edges(&r, EdgeType::Imports),
            set(&[
                ("m.py", "json"),
                ("m.py", "os"),
                ("m.py", "a.b"),
                ("m.py", "app"),
                ("m.py", "app.util"),
                ("m.py", "app.x"),
                ("m.py", "."),
                ("m.py", ".z"),
                ("m.py", ".."),
                ("m.py", "..w"),
                ("m.py", ".m"),
                ("m.py", ".m.p"),
                ("m.py", ".m.q"),
                ("m.py", "star"),
            ])
        );
        let submodules: BTreeSet<String> = r
            .edges
            .iter()
            .filter(|e| e.metadata.contains_key(crg_shape::PY_SUBMODULE_ATTR))
            .map(|e| e.to.clone())
            .collect();
        assert_eq!(
            submodules,
            names(&["app.util", "app.x", ".z", "..w", ".m.p", ".m.q"])
        );
    }
}
