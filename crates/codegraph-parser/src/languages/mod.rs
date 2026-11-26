// ABOUTME: Language extractor modules and shared infrastructure
// ABOUTME: Provides unified extraction interface for all supported languages

pub mod crg_shape;
pub mod extractor_utils;
pub mod javascript;
pub mod python;
pub mod rust;
// Revolutionary universal language support
pub mod cpp;
pub mod csharp;
pub mod dart;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod php;
pub mod ruby;
pub mod swift;
pub mod test_detection;

use codegraph_core::{EdgeType, ExtractionResult, Language};
use tree_sitter::Tree;

#[derive(Debug, Clone, Copy)]
pub struct ExtractionPolicy {
    pub uses: bool,
    pub references: bool,
}

impl Default for ExtractionPolicy {
    fn default() -> Self {
        Self {
            uses: true,
            references: true,
        }
    }
}

impl ExtractionPolicy {
    pub fn allows(self, edge: &EdgeType) -> bool {
        match edge {
            EdgeType::Uses => self.uses,
            EdgeType::References => self.references,
            _ => true,
        }
    }
}

/// Trait for language-specific AST extractors
///
/// All language extractors implement this trait to provide unified
/// extraction of code nodes and relationship edges.
pub trait LanguageExtractor {
    /// Extract nodes and edges in a single AST traversal
    fn extract_with_edges(tree: &Tree, content: &str, file_path: &str) -> ExtractionResult;

    /// List of edge types this extractor can produce
    fn supported_edge_types() -> &'static [EdgeType];

    /// Language identifier
    fn language() -> Language;
}

// Re-export extractors for convenience
pub use cpp::CppExtractor;
pub use csharp::CSharpExtractor;
pub use go::GoExtractor;
pub use java::JavaExtractor;
pub use javascript::TypeScriptExtractor;
pub use php::PhpExtractor;
pub use python::PythonExtractor;
pub use ruby::RubyExtractor;
pub use rust::RustExtractor;
pub use swift::SwiftExtractor;

/// Unified extraction dispatch for all supported languages
///
/// Routes to the appropriate language extractor based on the Language enum.
/// Returns None for unsupported languages.
pub fn extract_for_language(
    language: &Language,
    tree: &Tree,
    content: &str,
    file_path: &str,
) -> Option<ExtractionResult> {
    extract_for_language_with_policy(language, tree, content, file_path, Default::default())
}

pub fn extract_for_language_with_policy(
    language: &Language,
    tree: &Tree,
    content: &str,
    file_path: &str,
    policy: ExtractionPolicy,
) -> Option<ExtractionResult> {
    let rel_path = crg_shape::default_rel_path(file_path);
    extract_for_language_at(language, tree, content, file_path, &rel_path, policy)
}

/// Extraction for `file_path`, whose path relative to the project root is `rel_path`.
pub fn extract_for_language_at(
    language: &Language,
    tree: &Tree,
    content: &str,
    file_path: &str,
    rel_path: &str,
    policy: ExtractionPolicy,
) -> Option<ExtractionResult> {
    let mut result = extract_unshaped(language, tree, content, file_path, rel_path, policy)?;
    let line_count = crg_shape::line_count(content);
    crg_shape::shape_for_crg(
        &mut result,
        file_path,
        rel_path,
        language.clone(),
        line_count,
    );
    Some(result)
}

fn extract_unshaped(
    language: &Language,
    tree: &Tree,
    content: &str,
    file_path: &str,
    rel_path: &str,
    policy: ExtractionPolicy,
) -> Option<ExtractionResult> {
    match language {
        Language::Rust => Some(RustExtractor::extract_with_policy(
            tree, content, file_path, policy,
        )),
        // JavaScript and TSX use the TypeScript extractor; the registry picks the grammar.
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            Some(TypeScriptExtractor::extract_with_edges(
                tree,
                content,
                file_path,
                language.clone(),
                test_detection::is_test_file(rel_path),
            ))
        }
        Language::Python => Some(PythonExtractor::extract_with_policy(
            tree, content, file_path, policy,
        )),
        Language::Go => Some(<GoExtractor as LanguageExtractor>::extract_with_edges(
            tree, content, file_path,
        )),
        Language::Java => Some(<JavaExtractor as LanguageExtractor>::extract_with_edges(
            tree, content, file_path,
        )),
        Language::Cpp => Some(<CppExtractor as LanguageExtractor>::extract_with_edges(
            tree, content, file_path,
        )),
        Language::Swift => Some(<SwiftExtractor as LanguageExtractor>::extract_with_edges(
            tree, content, file_path,
        )),
        Language::CSharp => Some(<CSharpExtractor as LanguageExtractor>::extract_with_edges(
            tree, content, file_path,
        )),
        Language::Ruby => Some(<RubyExtractor as LanguageExtractor>::extract_with_edges(
            tree, content, file_path,
        )),
        Language::Php => Some(<PhpExtractor as LanguageExtractor>::extract_with_edges(
            tree, content, file_path,
        )),
        Language::Dart => Some(
            <dart::DartExtractor as LanguageExtractor>::extract_with_edges(
                tree, content, file_path,
            ),
        ),
        Language::Kotlin => Some(
            <kotlin::KotlinExtractor as LanguageExtractor>::extract_with_edges(
                tree, content, file_path,
            ),
        ),
        // Languages without dedicated extractors yet
        _ => None,
    }
}
