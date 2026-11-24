// ABOUTME: End-to-end tests for the CodeGraph MCP stdio server tool surface.
// ABOUTME: Validates tool behavior via an rmcp client against a real server process.
/// E2E Tests for CodeGraph MCP Server Tools
///
/// Tests the graph tool surface using the official rmcp client library.
use anyhow::{Result, anyhow};
use rmcp::{RoleClient, ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::Value;
use serde_json::json;
use std::path::PathBuf;
use std::process::{Command as StdCommand, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

/// Test configuration and utilities
struct TestConfig {
    tool_timeout: Duration,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            tool_timeout: Duration::from_secs(60),
        }
    }
}

static CODEGRAPH_BIN: OnceLock<PathBuf> = OnceLock::new();

fn workspace_root_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .expect("Expected CARGO_MANIFEST_DIR to be under <workspace>/crates/<crate>")
}

fn target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root_dir().join("target"))
}

fn codegraph_bin_path() -> PathBuf {
    let exe = if cfg!(windows) {
        "codegraph.exe"
    } else {
        "codegraph"
    };
    target_dir().join("fast-dev").join(exe)
}

fn ensure_codegraph_bin() -> PathBuf {
    CODEGRAPH_BIN
        .get_or_init(|| {
            let workspace_root = workspace_root_dir();
            let output = StdCommand::new("cargo")
                .current_dir(&workspace_root)
                .args([
                    "build",
                    "--profile",
                    "fast-dev",
                    "-q",
                    "-p",
                    "codegraph-mcp-server",
                    "--bin",
                    "codegraph",
                ])
                .stdin(Stdio::null())
                .output()
                .expect("Failed to invoke cargo build for codegraph-mcp-server");

            if !output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                panic!(
                    "Failed to build codegraph-mcp-server binary.\nstdout:\n{}\nstderr:\n{}",
                    stdout, stderr
                );
            }

            let bin = codegraph_bin_path();
            assert!(
                bin.exists(),
                "Expected built binary at '{}', but it does not exist",
                bin.display()
            );
            bin
        })
        .clone()
}

/// Start CodeGraph MCP server as a child process in an empty project directory, so the
/// lazily opened store never touches the workspace.
async fn start_mcp_server_in(
    project: &std::path::Path,
) -> Result<rmcp::service::RunningService<RoleClient, ()>> {
    let codegraph_bin = ensure_codegraph_bin();

    let mut cmd = TokioCommand::new(codegraph_bin);
    cmd.args(["start", "stdio"])
        .current_dir(project)
        .env_remove("CODEGRAPH_PROJECT_ID")
        .env("RUST_LOG", "error") // Minimize log noise during testing
        .env("CODEGRAPH_DEBUG", "0"); // Force-disable debug log files in tests

    let (transport, _stderr) = TokioChildProcess::builder(cmd)
        .stderr(Stdio::null())
        .spawn()?;

    let service = ().serve(transport).await?;
    Ok(service)
}

async fn start_mcp_server() -> Result<rmcp::service::RunningService<RoleClient, ()>> {
    let project = std::env::temp_dir().join(format!("codegraph-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&project)?;
    start_mcp_server_in(&project).await
}

#[tokio::test]
async fn test_tool_discovery() -> Result<()> {
    println!("🔍 Testing MCP tool discovery...");

    let service = start_mcp_server().await?;
    let tools = service.list_tools(Default::default()).await?;

    let mut discovered: Vec<String> = tools
        .tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    discovered.sort();
    let mut expected = vec![
        "calculate_coupling_metrics",
        "detect_circular_dependencies",
        "find_complexity_hotspots",
        "find_large_functions",
        "find_nodes_by_name",
        "build_or_update_graph",
        "get_hub_nodes",
        "get_reverse_dependencies",
        "get_transitive_dependencies",
        "list_graph_stats",
        "query_graph",
        "traverse_graph",
        "get_impact_radius",
        "detect_changes",
        "get_minimal_context",
        "get_review_context",
        "search_nodes",
        "trace_call_chain",
        "list_flows",
        "get_flow",
        "get_affected_flows",
        "list_communities",
        "get_community",
        "get_architecture_overview",
        "get_bridge_nodes",
        "get_knowledge_gaps",
        "get_surprising_connections",
    ];
    expected.sort();
    assert_eq!(discovered, expected);

    for tool in &tools.tools {
        // MCP requires an object schema; parameterless tools may omit "properties".
        assert_eq!(
            tool.input_schema.get("type").and_then(|t| t.as_str()),
            Some("object"),
            "Tool '{}' must publish a parameter schema",
            tool.name
        );
    }

    service.cancel().await?;
    println!("🎉 Tool discovery test passed!");
    Ok(())
}

#[tokio::test]
async fn test_graph_tool_runs_against_an_empty_project() -> Result<()> {
    println!("🧪 Testing a graph tool call against an unindexed project...");

    let project = tempfile::tempdir()?;
    let service = start_mcp_server_in(project.path()).await?;
    let config = TestConfig::default();

    let response = timeout(
        config.tool_timeout,
        service.call_tool(
            CallToolRequestParams::new("find_nodes_by_name")
                .with_arguments(json!({"name": "main"}).as_object().unwrap().clone()),
        ),
    )
    .await
    .map_err(|_| anyhow!("Timed out calling find_nodes_by_name"))??;

    assert_ne!(response.is_error, Some(true), "{response:?}");
    let text = response
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.clone()))
        .ok_or_else(|| anyhow!("Expected a text result"))?;
    let result: Value = serde_json::from_str(&text)?;
    assert_eq!(result["tool"], "find_nodes_by_name");
    assert_eq!(result["result"], json!([]));

    service.cancel().await?;
    println!("🎉 Empty-project tool call test passed!");
    Ok(())
}

#[tokio::test]
async fn test_error_conditions() -> Result<()> {
    println!("⚠️ Testing error condition handling...");

    let service = start_mcp_server().await?;

    // Test 1: Invalid parameters
    println!("🧪 Testing invalid parameters...");
    let result = service
        .call_tool(
            CallToolRequestParams::new("find_nodes_by_name").with_arguments(
                json!({
                    "invalid_param": "test"
                    // Missing required "name" parameter
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;

    // Should handle gracefully (either error or empty response)
    match result {
        Ok(response) => {
            assert_eq!(
                response.is_error,
                Some(true),
                "Invalid arguments must be rejected"
            );
            println!("✅ Tool handled invalid parameters gracefully");
        }
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("name") || msg.contains("Invalid") || msg.contains("invalid"),
                "Unexpected error for invalid parameters: {}",
                msg
            );
            println!(
                "✅ Tool properly returned error for invalid parameters: {}",
                e
            );
        }
    }

    // Test 2: Non-existent tool
    println!("🧪 Testing non-existent tool...");
    let result = service
        .call_tool(CallToolRequestParams::new("non_existent_tool"))
        .await;

    assert!(result.is_err(), "Should error for non-existent tool");
    println!("✅ Properly rejected non-existent tool");

    // Test 3: Invalid parameter types
    println!("🧪 Testing invalid parameter types...");
    let result = service
        .call_tool(
            CallToolRequestParams::new("find_nodes_by_name").with_arguments(
                json!({
                    "name": "type mismatch test",
                    "limit": "not-a-number"
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await;

    // Should reject invalid types during request decoding
    match result {
        Ok(response) => assert_eq!(
            response.is_error,
            Some(true),
            "Invalid parameter types must be rejected"
        ),
        Err(e) => println!("✅ Tool rejected invalid parameter types: {}", e),
    }

    service.cancel().await?;
    println!("🎉 Error condition test passed!");
    Ok(())
}

#[tokio::test]
async fn test_server_health() -> Result<()> {
    println!("🏥 Testing MCP server health and responsiveness...");

    let service = start_mcp_server().await?;

    // Test basic connectivity
    let tools = service.list_tools(Default::default()).await?;
    assert!(!tools.tools.is_empty(), "Server should expose tools");

    // Test server info
    let info = service
        .peer_info()
        .expect("Server should provide peer info after initialization");
    println!(
        "📋 Server info: {:?}",
        info.server_info
            .as_ref()
            .map(|server| server.name.as_str())
            .unwrap_or("unknown")
    );

    service.cancel().await?;
    println!("🎉 Server health test passed!");
    Ok(())
}

/// Performance benchmark test
#[tokio::test]
async fn test_performance_benchmarks() -> Result<()> {
    println!("⚡ Testing performance benchmarks...");

    let service = start_mcp_server().await?;

    // Test basic responsiveness via tool listing
    let start_time = std::time::Instant::now();

    let _tools = service.list_tools(Default::default()).await?;

    let list_tools_time = start_time.elapsed();
    println!("⏱️ list_tools time: {:?}", list_tools_time);

    // Listing tools should be fast (< 5 seconds)
    assert!(
        list_tools_time < Duration::from_secs(5),
        "Tool discovery should be fast"
    );

    service.cancel().await?;
    println!("🎉 Performance benchmark test passed!");
    Ok(())
}
