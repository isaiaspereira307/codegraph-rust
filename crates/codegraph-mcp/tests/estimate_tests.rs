// ABOUTME: Ensures repository estimation helpers expose correct calculations.
// ABOUTME: Validates symbol indexing used by the CLI estimate and the indexer.

use codegraph_core::{CodeNode, Language, Location, NodeType};
use codegraph_mcp::estimation::build_symbol_index;

fn sample_location() -> Location {
    Location {
        file_path: "src/lib.rs".to_string(),
        line: 1,
        column: 0,
        end_line: Some(1),
        end_column: Some(10),
    }
}

fn make_node(name: &str, node_type: NodeType) -> CodeNode {
    CodeNode::new(
        name,
        Some(node_type),
        Some(Language::Rust),
        sample_location(),
    )
}

#[test]
fn symbol_index_records_expected_aliases() {
    let mut import_node = make_node("chrono::Utc", NodeType::Import);
    import_node
        .metadata
        .attributes
        .insert("qualified_name".into(), "./foo.rs::chrono::Utc".into());
    import_node
        .metadata
        .attributes
        .insert("method_of".into(), "chrono".into());
    import_node
        .metadata
        .attributes
        .insert("implements_trait".into(), "TimeZone".into());

    let mut struct_node = make_node("MyStruct", NodeType::Struct);
    struct_node
        .metadata
        .attributes
        .insert("qualified_name".into(), "crate::MyStruct".into());

    let symbols = build_symbol_index(&[import_node, struct_node]);

    assert!(symbols.contains_key("chrono::Utc"));
    assert!(symbols.contains_key("./foo.rs::chrono::Utc"));
    assert!(symbols.contains_key("Import::chrono::Utc"));
    assert!(symbols.contains_key("src/lib.rs::chrono::Utc"));
    assert!(symbols.contains_key("Utc"));
    assert!(symbols.contains_key("chrono::chrono::Utc"));
    assert!(symbols.contains_key("TimeZone::chrono::Utc"));
    assert!(symbols.contains_key("MyStruct"));
    assert!(symbols.contains_key("Struct::MyStruct"));
}
