// ABOUTME: One-shot CLI access to the graph analysis tools.
// ABOUTME: Writes results to stdout and diagnostics to stderr without an MCP server.

use crate::{CodeGraphMCPServer, agent_hooks::CLI_INSTRUCTIONS};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use codegraph_mcp_tools::GraphToolExecutor;
use serde_json::Value;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Subcommand)]
pub enum ToolCommand {
    /// Run one graph tool, e.g. `codegraph tool run find_nodes_by_name --params '{"name":"Config"}'`.
    Run {
        /// Tool name; see `codegraph tool list`.
        name: String,
        /// Tool parameters as a JSON object.
        #[arg(long, default_value = "{}")]
        params: String,
        #[command(flatten)]
        target: ToolTarget,
    },
    /// List the available tools with their parameter schemas.
    List,
    /// Print CLI guidance without loading configuration or opening the database.
    Instructions,
}

#[derive(Debug, Clone, Args)]
pub struct ToolTarget {
    /// Indexed project root; configuration is loaded from this directory.
    #[arg(long)]
    pub project: Option<PathBuf>,
    /// Override CODEGRAPH_PROJECT_ID for projects indexed with a custom identifier.
    #[arg(long)]
    pub project_id: Option<String>,
    /// Whole-command deadline in seconds, including database setup.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout_secs: u64,
}

/// Execute a CLI command, preparing process configuration before starting workers.
///
/// # Safety
/// The caller must be the single-threaded process entry point; no other threads may
/// read the environment while configuration overrides and dotenv are applied.
pub unsafe fn run(command: &ToolCommand, verbose: bool, config_path: Option<&Path>) -> Result<()> {
    let (name, params, target) = match command {
        ToolCommand::Instructions => {
            println!("{CLI_INSTRUCTIONS}");
            return Ok(());
        }
        ToolCommand::List => {
            let schemas = GraphToolExecutor::get_tool_schemas();
            println!("{}", serde_json::to_string_pretty(&schemas)?);
            return Ok(());
        }
        ToolCommand::Run {
            name,
            params,
            target,
        } => (name, params, target),
    };
    let result = (|| {
        ensure_known_tool(name)?;
        let params = parse_params(params)?;
        // SAFETY: The caller guarantees this runs before any worker threads exist.
        unsafe { run_prepared(name, params, target, verbose, config_path) }
    })();
    let mut stdout = io::stdout().lock();
    match result {
        Ok(response) => {
            writeln!(stdout, "{response}")?;
            Ok(())
        }
        Err(error) => {
            let response = serde_json::json!({"error": {
                "tool": name, "message": format!("{error:#}")
            }});
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
            Err(error).with_context(|| format!("{name} failed"))
        }
    }
}

/// Runs one graph tool and returns its JSON result without printing, so other commands
/// (e.g. `pr-report`) share the exact setup, timeout and executor path of `tool run`.
///
/// # Safety
/// Same contract as [`run`]: call from the single-threaded process entry point.
pub unsafe fn run_to_value(
    name: &str,
    params: Value,
    target: &ToolTarget,
    verbose: bool,
    config_path: Option<&Path>,
) -> Result<Value> {
    ensure_known_tool(name)?;
    // SAFETY: forwarded from the caller's guarantee.
    unsafe { run_prepared(name, params, target, verbose, config_path) }
}

fn ensure_known_tool(name: &str) -> Result<()> {
    if GraphToolExecutor::get_tool_names()
        .iter()
        .all(|known| known != name)
    {
        bail!("Unknown tool: {name}; see `codegraph tool list`");
    }
    Ok(())
}

unsafe fn run_prepared(
    name: &str,
    params: Value,
    target: &ToolTarget,
    verbose: bool,
    config_path: Option<&Path>,
) -> Result<Value> {
    // SAFETY: forwarded from the caller's guarantee.
    unsafe { prepare_environment(target, config_path) }?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(execute(name, params, target, verbose))
}

async fn execute(name: &str, params: Value, target: &ToolTarget, verbose: bool) -> Result<Value> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(if verbose { "info" } else { "warn" })
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .with_ansi(false)
        .try_init();

    let server = CodeGraphMCPServer::new();
    tokio::time::timeout(
        Duration::from_secs(target.timeout_secs),
        server.call_graph_tool(name, params),
    )
    .await
    .with_context(|| format!("Tool timed out after {} seconds", target.timeout_secs))?
    .map_err(|error| anyhow::anyhow!("{}", error.message))
}

fn parse_params(params: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(params).context("--params must be valid JSON")?;
    if !value.is_object() {
        bail!("--params must be a JSON object");
    }
    Ok(value)
}

unsafe fn prepare_environment(target: &ToolTarget, config_path: Option<&Path>) -> Result<()> {
    // Resolve relative config paths before changing to the selected project.
    let config_path = config_path
        .map(std::fs::canonicalize)
        .transpose()
        .context("Cannot resolve --config file")?;
    if let Some(project) = &target.project {
        std::env::set_current_dir(project)
            .with_context(|| format!("Cannot open project {}", project.display()))?;
    }
    // SAFETY: The caller guarantees configuration precedes worker startup.
    unsafe { codegraph_core::config_manager::ConfigManager::initialize_environment() };
    if let Some(project_id) = &target.project_id {
        if project_id.trim().is_empty() {
            bail!("--project-id must not be blank");
        }
        // SAFETY: Called from the single-threaded CLI before creating the async runtime.
        unsafe { std::env::set_var("CODEGRAPH_PROJECT_ID", project_id) };
    }
    if let Some(config_path) = config_path {
        // SAFETY: Called from the single-threaded CLI before creating the async runtime.
        unsafe { std::env::set_var("CODEGRAPH_CONFIG_PATH", config_path) };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: ToolCommand,
    }

    #[test]
    fn run_takes_a_tool_name_and_json_params() {
        let parsed = TestCli::try_parse_from([
            "tool",
            "run",
            "find_nodes_by_name",
            "--params",
            r#"{"name":"Config"}"#,
        ])
        .unwrap();
        let ToolCommand::Run {
            name,
            params,
            target,
        } = parsed.command
        else {
            panic!("expected run");
        };
        assert_eq!(name, "find_nodes_by_name");
        assert_eq!(parse_params(&params).unwrap()["name"], "Config");
        assert_eq!(target.timeout_secs, 120);
    }

    #[test]
    fn params_must_be_a_json_object() {
        assert!(parse_params("{}").is_ok());
        assert!(parse_params("[]").is_err());
        assert!(parse_params("not json").is_err());
        assert!(TestCli::try_parse_from(["tool", "run", "x", "--timeout-secs", "0"]).is_err());
    }
}
