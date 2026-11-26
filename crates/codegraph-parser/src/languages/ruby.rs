use codegraph_core::{
    CodeNode, EdgeRelationship, EdgeType, ExtractionResult, Language, Location, NodeId, NodeType,
    Span,
};
use std::collections::HashMap;
use std::path::Path;
use tree_sitter::{Node, Tree, TreeCursor};

/// Advanced Ruby AST extractor for dynamic programming intelligence.
///
/// Extracts:
/// - classes, modules, methods, constants
/// - metaprogramming patterns (define_method, class_eval, etc.)
/// - Rails patterns (controllers, models, migrations)
/// - blocks, lambdas, and functional patterns
/// - mixins, includes, and module composition
/// - attr_accessor and dynamic property definitions
/// - gem dependencies and require statements
///
/// Notes:
/// - Optimized for Ruby on Rails development patterns
/// - Captures dynamic and metaprogramming constructs
/// - Handles Ruby's flexible syntax and duck typing
/// - Understands Rails conventions and patterns
pub struct RubyExtractor;

#[derive(Default, Clone)]
struct RubyContext {
    module_path: Vec<String>,
    current_class: Option<String>,
    current_module: Option<String>,
    is_rails_file: bool,
}

impl RubyExtractor {
    pub fn extract(tree: &Tree, content: &str, file_path: &str) -> Vec<CodeNode> {
        let mut collector = RubyCollector::new(content, file_path);
        let mut cursor = tree.walk();

        // Detect Rails patterns from file path
        let is_rails = file_path.contains("/app/")
            || file_path.contains("/config/")
            || file_path.contains("/db/migrate");

        let mut ctx = RubyContext::default();
        ctx.is_rails_file = is_rails;

        collector.walk(&mut cursor, ctx);
        collector.into_nodes()
    }

    /// Unified extraction of nodes AND edges in single AST traversal
    pub fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        let mut collector = RubyCollector::new(content, file_path);
        let mut cursor = tree.walk();

        // Detect Rails patterns from file path
        let is_rails = file_path.contains("/app/")
            || file_path.contains("/config/")
            || file_path.contains("/db/migrate");

        let mut ctx = RubyContext::default();
        ctx.is_rails_file = is_rails;

        collector.walk(&mut cursor, ctx);
        collector.into_result()
    }
}

impl super::LanguageExtractor for RubyExtractor {
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult {
        RubyExtractor::extract_with_edges(tree, content, file_path)
    }

    fn supported_edge_types() -> &'static [EdgeType] {
        &[EdgeType::Imports, EdgeType::Calls, EdgeType::Extends]
    }

    fn language() -> Language {
        Language::Ruby
    }
}

struct RubyCollector<'a> {
    content: &'a str,
    file_path: &'a str,
    nodes: Vec<CodeNode>,
    edges: Vec<EdgeRelationship>,
    current_function_id: Option<NodeId>,
    _current_class_id: Option<NodeId>,
}

impl<'a> RubyCollector<'a> {
    fn new(content: &'a str, file_path: &'a str) -> Self {
        Self {
            content,
            file_path,
            nodes: Vec::new(),
            edges: Vec::new(),
            current_function_id: None,
            _current_class_id: None,
        }
    }

    fn into_nodes(self) -> Vec<CodeNode> {
        self.nodes
    }

    fn into_result(self) -> ExtractionResult {
        ExtractionResult {
            nodes: self.nodes,
            edges: self.edges,
        }
    }

    fn span_for(&self, node: &Node) -> Span {
        Span {
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        }
    }

    fn walk(&mut self, cursor: &mut TreeCursor, mut ctx: RubyContext) {
        let node = cursor.node();

        match node.kind() {
            // Ruby Classes
            "class" => {
                if let Some(name) = self.child_text_by_field(node, "name") {
                    let loc = self.location(&node);
                    let content_text = self.node_text(&node);
                    let mut code = CodeNode::new(
                        name.clone(),
                        Some(NodeType::Class),
                        Some(Language::Ruby),
                        loc,
                    )
                    .with_content(content_text.clone());

                    // Detect inheritance
                    if let Some(superclass) = self.child_text_by_field(node, "superclass") {
                        code.metadata
                            .attributes
                            .insert("superclass".into(), superclass);
                    }

                    // Detect Rails patterns
                    if ctx.is_rails_file {
                        if name.ends_with("Controller") {
                            code.metadata
                                .attributes
                                .insert("rails_pattern".into(), "controller".into());
                        } else if content_text.contains("< ApplicationRecord") {
                            code.metadata
                                .attributes
                                .insert("rails_pattern".into(), "model".into());
                        } else if content_text.contains("< ActiveRecord::Migration") {
                            code.metadata
                                .attributes
                                .insert("rails_pattern".into(), "migration".into());
                        }
                    }

                    code.metadata
                        .attributes
                        .insert("kind".into(), "class".into());
                    self.nodes.push(code);
                    ctx.current_class = Some(name);
                }
            }

            // Ruby Modules. code-review-graph indexes both `module` and `class` as Class
            // nodes, and the module participates in the dotted scope of its members.
            "module" => {
                if let Some(name) = self.child_text_by_field(node, "name") {
                    let loc = self.location(&node);
                    let mut code = CodeNode::new(
                        name.clone(),
                        Some(NodeType::Class),
                        Some(Language::Ruby),
                        loc,
                    )
                    .with_content(self.node_text(&node));

                    code.metadata
                        .attributes
                        .insert("kind".into(), "module".into());
                    self.nodes.push(code);
                    ctx.module_path.push(name.clone());
                    ctx.current_module = Some(name);
                }
            }

            // Ruby Methods
            "method" => {
                if let Some(name) = self.child_text_by_field(node, "name") {
                    let loc = self.location(&node);
                    let content_text = self.node_text(&node);
                    let mut code = CodeNode::new(
                        name.clone(),
                        Some(NodeType::Function),
                        Some(Language::Ruby),
                        loc,
                    )
                    .with_content(content_text.clone())
                    .with_complexity(
                        crate::complexity::calculate_cyclomatic_complexity(&node, self.content),
                    );
                    code.span = Some(self.span_for(&node));

                    // Detect class vs instance methods
                    if content_text.starts_with("def self.") {
                        code.metadata
                            .attributes
                            .insert("method_type".into(), "class".into());
                    } else {
                        code.metadata
                            .attributes
                            .insert("method_type".into(), "instance".into());
                    }

                    // Detect Rails action methods
                    if ctx.is_rails_file
                        && ctx
                            .current_class
                            .as_ref()
                            .map_or(false, |c| c.ends_with("Controller"))
                    {
                        if matches!(
                            name.as_str(),
                            "index" | "show" | "new" | "create" | "edit" | "update" | "destroy"
                        ) {
                            code.metadata
                                .attributes
                                .insert("rails_action".into(), "true".into());
                        }
                    }

                    // Detect metaprogramming patterns
                    if content_text.contains("define_method") {
                        code.metadata
                            .attributes
                            .insert("metaprogramming".into(), "define_method".into());
                    }

                    code.metadata
                        .attributes
                        .insert("kind".into(), "method".into());
                    if let Some(ref current_class) = ctx.current_class {
                        code.metadata
                            .attributes
                            .insert("parent_class".into(), current_class.clone());
                    }

                    // Track current function for call edge attribution
                    self.current_function_id = Some(code.id);
                    self.nodes.push(code);
                }
            }

            // CRG emits no node for a bare Ruby constant: class and module names are already
            // represented by their declaration nodes, and other constants are references.
            "call" => {
                let call_text = self.node_text(&node);
                if call_text.starts_with("attr_") {
                    let loc = self.location(&node);
                    let mut code = CodeNode::new(
                        call_text.clone(),
                        Some(NodeType::Variable),
                        Some(Language::Ruby),
                        loc,
                    )
                    .with_content(call_text.clone());

                    // Detect specific attr types
                    if call_text.starts_with("attr_accessor") {
                        code.metadata
                            .attributes
                            .insert("attr_type".into(), "accessor".into());
                    } else if call_text.starts_with("attr_reader") {
                        code.metadata
                            .attributes
                            .insert("attr_type".into(), "reader".into());
                    } else if call_text.starts_with("attr_writer") {
                        code.metadata
                            .attributes
                            .insert("attr_type".into(), "writer".into());
                    }

                    code.metadata
                        .attributes
                        .insert("kind".into(), "attribute".into());
                    self.nodes.push(code);
                } else if call_text.starts_with("require") {
                    let loc = self.location(&node);
                    let mut code = CodeNode::new(
                        call_text.clone(),
                        Some(NodeType::Import),
                        Some(Language::Ruby),
                        loc,
                    )
                    .with_content(call_text.clone());
                    code.span = Some(self.span_for(&node));

                    // Extract the required path from require 'path' or require_relative 'path'
                    let is_relative = call_text.contains("require_relative");
                    let import_type = if is_relative {
                        "require_relative"
                    } else {
                        "require"
                    };

                    // Try to extract the path argument
                    let path = if let Some(args) = node.child_by_field_name("arguments") {
                        self.node_text(&args)
                            .trim_matches(|c| c == '(' || c == ')' || c == '\'' || c == '"')
                            .to_string()
                    } else {
                        call_text.clone()
                    };
                    // `require_relative` is anchored at the importer's directory and CRG keeps
                    // the `.rb` suffix in the edge target.
                    let path = if is_relative {
                        self.require_relative_path(&path)
                    } else {
                        path
                    };

                    // Create import edge
                    let edge = EdgeRelationship {
                        from: code.id,
                        to: path.clone(),
                        edge_type: EdgeType::Imports,
                        metadata: {
                            let mut meta = HashMap::new();
                            meta.insert("import_type".to_string(), import_type.to_string());
                            meta.insert("source_file".to_string(), self.file_path.to_string());
                            if is_relative {
                                meta.insert("relative".to_string(), "true".to_string());
                            }
                            if in_method(&node) {
                                meta.insert(
                                    super::crg_shape::IMPORT_CONTEXT_ATTR.to_string(),
                                    super::crg_shape::IMPORT_DEFERRED.to_string(),
                                );
                            }
                            meta
                        },
                        span: Some(self.span_for(&node)),
                    };
                    self.edges.push(edge);

                    code.metadata
                        .attributes
                        .insert("kind".into(), "require".into());
                    code.metadata.attributes.insert("path".into(), path);
                    self.nodes.push(code);
                } else if let Some(current_fn) = self.current_function_id {
                    // Generic method call - create call edge
                    if let Some(method) = node.child_by_field_name("method") {
                        let method_name = self.node_text(&method);
                        if !method_name.is_empty() {
                            let edge = EdgeRelationship {
                                from: current_fn,
                                to: method_name,
                                edge_type: EdgeType::Calls,
                                metadata: {
                                    let mut meta = HashMap::new();
                                    meta.insert("call_type".to_string(), "ruby_call".to_string());
                                    meta
                                },
                                span: Some(self.span_for(&node)),
                            };
                            self.edges.push(edge);
                        }
                    }
                }
            }

            _ => {}
        }

        // Recursively walk children
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

    /// Resolves a `require_relative` argument like CRG: from the importer's directory, with
    /// the Ruby extension retained (and added when the source omitted it).
    fn require_relative_path(&self, path: &str) -> String {
        let path = path.trim();
        let with_extension = if path.ends_with(".rb") {
            path.to_string()
        } else {
            format!("{path}.rb")
        };
        let parent = Path::new(self.file_path)
            .parent()
            .unwrap_or_else(|| Path::new(""));
        super::crg_shape::default_rel_path(&parent.join(with_extension).to_string_lossy())
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

/// CRG `_import_context` for Ruby: a `require` inside a method only runs when it is called.
fn in_method(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "method" | "singleton_method") {
            return true;
        }
        current = parent.parent();
    }
    false
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
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        extract_for_language_at(
            &Language::Ruby,
            &tree,
            source,
            "app.rb",
            "app.rb",
            ExtractionPolicy::default(),
        )
        .unwrap()
    }

    #[test]
    fn ruby_modules_scope_classes_and_methods_like_crg() {
        let result = extract("module Demo\n  class App\n    def run\n    end\n  end\nend\n");
        let names: Vec<_> = result
            .nodes
            .iter()
            .filter_map(|n| n.metadata.attributes.get("qualified_name").cloned())
            .collect();
        assert!(names.iter().any(|q| q == "app.rb::Demo"));
        assert!(names.iter().any(|q| q == "app.rb::Demo.App"));
        assert!(names.iter().any(|q| q == "app.rb::Demo.App.run"));
        // CRG indexes a class name as the scope's Class node, never as a separate constant.
        assert!(!names.iter().any(|q| q.contains("@L")));
    }

    /// CRG `_import_context` for Ruby: a `require` inside a method runs late.
    #[test]
    fn ruby_require_in_a_method_is_deferred() {
        let result = extract(
            "require 'top'\nclass A\n  def run\n    require 'lazy'\n  end\n  \
             def self.boot\n    require 'boot'\n  end\nend\n",
        );
        let contexts: Vec<(String, Option<String>)> = result
            .edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Imports)
            .map(|e| {
                let context = e
                    .metadata
                    .get(super::super::crg_shape::IMPORT_CONTEXT_ATTR)
                    .cloned();
                (e.to.clone(), context)
            })
            .collect();
        let deferred = Some("deferred".to_string());
        assert_eq!(
            contexts,
            [
                ("top".to_string(), None),
                ("lazy".to_string(), deferred.clone()),
                ("boot".to_string(), deferred),
            ]
        );
    }

    #[test]
    fn ruby_require_relative_keeps_the_rb_extension() {
        let result = extract("require_relative 'lib/helper'\n");
        assert!(
            result
                .edges
                .iter()
                .any(|e| { e.edge_type == EdgeType::Imports && e.to == "lib/helper.rb" })
        );
    }
}
