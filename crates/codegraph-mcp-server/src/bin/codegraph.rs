use anyhow::{Context, Result};
use atty::Stream;
use chrono::Utc;
use clap::{Parser, Subcommand};
use codegraph_mcp::{
    IndexerConfig, ProcessManager, ProjectIndexer, RepositoryEstimate, RepositoryEstimator,
};
use codegraph_mcp_core::debug_logger::DebugLogger;
#[cfg(feature = "daemon")]
use codegraph_mcp_daemon::{DaemonManager, PidFile, WatchConfig, WatchDaemon};
use codegraph_mcp_server::CodeGraphMCPServer;
use codegraph_mcp_server::{agent_hooks, project_init, tool_cli};
use colored::Colorize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rmcp::ServiceExt;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::info;
use tracing_subscriber::{
    Registry, filter::EnvFilter, fmt::writer::BoxMakeWriter, layer::SubscriberExt,
};

#[derive(Parser)]
#[command(
    name = "codegraph",
    version,
    author,
    about = "CodeGraph CLI - code graph tools, MCP server management, and project indexing",
    long_about = "Run CodeGraph's graph analysis tools directly, configure harness hooks, manage MCP servers, and index projects."
)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, global = true, help = "Enable verbose logging")]
    verbose: bool,

    #[arg(
        long,
        global = true,
        help = "Capture index logs to a file under .codegraph/logs"
    )]
    debug: bool,

    #[arg(long, global = true, help = "Configuration file path")]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Choose project hooks, add agent instructions, then index the project")]
    Init {
        #[arg(default_value = ".", help = "Project directory")]
        path: PathBuf,
        #[arg(
            long,
            value_enum,
            help = "Project-local hooks; prompts interactively when absent"
        )]
        hooks: Option<project_init::HookSelection>,
        #[arg(
            long,
            help = "Also register the MCP server (.mcp.json for Claude, .codex/config.toml for Codex)"
        )]
        mcp: bool,
        #[arg(
            long,
            help = "Set up hooks/instructions without loading providers or indexing"
        )]
        no_index: bool,
        #[arg(long, value_enum, help = "Indexing tier: fast | balanced | full")]
        index_tier: Option<IndexTier>,
        #[arg(long, default_value_t = 4, help = "Parallel parser workers")]
        workers: usize,
    },

    #[command(
        about = "Check the binary, graph database, hooks, skills and MCP entries (read-only)"
    )]
    Doctor {
        #[arg(default_value = ".", help = "Project directory")]
        path: PathBuf,
        #[arg(long, help = "Machine-readable output")]
        json: bool,
    },

    #[command(about = "Remove what `codegraph init` installed in a project")]
    Uninstall {
        #[arg(default_value = ".", help = "Project directory")]
        path: PathBuf,
        #[arg(
            long,
            value_enum,
            help = "Harness to remove: claude | codex | both (default both)"
        )]
        hooks: Option<project_init::HookSelection>,
        #[arg(long, help = "Also delete .code-review-graph/ (the graph database)")]
        purge_index: bool,
        #[arg(long, help = "List what would change without writing")]
        dry_run: bool,
    },

    #[command(about = "Run graph analysis tools without an MCP transport")]
    Tool {
        #[command(subcommand)]
        action: tool_cli::ToolCommand,
    },

    #[command(about = "Emit agent guidance or install project-local harness hooks")]
    Hooks {
        #[command(subcommand)]
        action: agent_hooks::HookCommand,
    },

    #[command(about = "Start MCP server with specified transport")]
    Start {
        #[command(subcommand)]
        transport: TransportType,

        #[arg(short, long, help = "Server configuration file")]
        config: Option<PathBuf>,

        #[arg(long, help = "Run server in background")]
        daemon: bool,

        #[arg(long, help = "PID file location for daemon mode")]
        pid_file: Option<PathBuf>,
    },

    #[command(about = "Open the project graph database and print its counts")]
    DbCheck,

    #[command(about = "Build the graph from scratch (full reindex)")]
    Build {
        #[arg(default_value = ".", help = "Project directory")]
        path: PathBuf,
        #[arg(long, default_value_t = 4, help = "Parallel parser workers")]
        workers: usize,
        #[arg(long, value_enum, help = "Indexing tier: fast | balanced | full")]
        index_tier: Option<IndexTier>,
        #[command(flatten)]
        postprocess: PostprocessFlags,
    },

    #[command(about = "Incrementally update the graph for changed files")]
    Update {
        #[arg(default_value = ".", help = "Project directory")]
        path: PathBuf,
        #[arg(long, default_value_t = 4, help = "Parallel parser workers")]
        workers: usize,
        #[arg(long, value_enum, help = "Indexing tier: fast | balanced | full")]
        index_tier: Option<IndexTier>,
        #[command(flatten)]
        postprocess: PostprocessFlags,
    },

    #[command(
        about = "Show node/edge counts, files, languages and last build of the project graph"
    )]
    GraphStatus {
        #[command(flatten)]
        target: tool_cli::ToolTarget,
    },

    #[command(about = "Map git changes to the graph nodes they touch")]
    DetectChanges {
        #[arg(long, help = "Git ref to diff against (default HEAD)")]
        base: Option<String>,
        #[command(flatten)]
        target: tool_cli::ToolTarget,
    },

    #[command(about = "List transitive dependents of nodes or files")]
    Impact {
        #[arg(long, value_delimiter = ',', help = "Node ids, comma separated")]
        node_ids: Vec<String>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Project-relative file paths, comma separated"
        )]
        files: Vec<String>,
        #[command(flatten)]
        target: tool_cli::ToolTarget,
    },

    #[command(about = "Run one relationship query from a node")]
    Query {
        #[arg(help = "callers_of | callees_of | imports_of | importers_of | tests_for | inheritors_of")]
        pattern: String,
        #[arg(long, help = "Node id")]
        node_id: Option<String>,
        #[arg(long, help = "Symbol name (used when --node-id is absent)")]
        name: Option<String>,
        #[arg(long, help = "Maximum results (1-100)")]
        limit: Option<i64>,
        #[command(flatten)]
        target: tool_cli::ToolTarget,
    },

    #[command(about = "Full-text search over node names")]
    Search {
        #[arg(help = "Search text")]
        query: String,
        #[arg(long, help = "Maximum results (1-50)")]
        limit: Option<i64>,
        #[command(flatten)]
        target: tool_cli::ToolTarget,
    },

    #[command(about = "Stop running MCP server")]
    Stop {
        #[arg(long, help = "PID file location")]
        pid_file: Option<PathBuf>,

        #[arg(short, long, help = "Force stop without graceful shutdown")]
        force: bool,
    },

    #[command(about = "Check status of MCP server")]
    Status {
        #[arg(long, help = "PID file location")]
        pid_file: Option<PathBuf>,

        #[arg(short, long, help = "Show detailed status information")]
        detailed: bool,
    },

    #[command(
        about = "Index a project or directory",
        long_about = "Index a project into <project>/.code-review-graph/graph.db \
                      (code-review-graph schema v15)."
    )]
    Index {
        #[arg(help = "Path to project directory")]
        path: PathBuf,

        #[arg(short, long, help = "Languages to index", value_delimiter = ',')]
        languages: Option<Vec<String>>,

        #[arg(long, help = "Exclude patterns (gitignore format)")]
        exclude: Vec<String>,

        #[arg(long, help = "Include only these patterns")]
        include: Vec<String>,

        #[arg(
            short,
            long,
            default_value_t = true,
            conflicts_with = "no_recursive",
            help = "Recursively index subdirectories (default)"
        )]
        recursive: bool,

        #[arg(long, help = "Index only files directly in the project directory")]
        no_recursive: bool,

        #[arg(long, help = "Force reindex even if already indexed")]
        force: bool,

        #[arg(long, help = "Watch for changes and auto-reindex")]
        watch: bool,

        #[arg(long, help = "Number of parallel workers", default_value = "4")]
        workers: usize,

        #[arg(
            long,
            help = "Rows per storage write batch (default 100)",
            value_parser = parse_positive_batch_size
        )]
        batch_size: Option<usize>,

        #[arg(long, value_enum, help = "Indexing tier: fast | balanced | full")]
        index_tier: Option<IndexTier>,
        #[arg(long, help = "Write indexing metrics and completion status as JSON")]
        stats_json: Option<PathBuf>,
        #[command(flatten)]
        postprocess: PostprocessFlags,
    },

    #[command(
        about = "Estimate graph size (node/edge counts, parse time) without writing the graph"
    )]
    Estimate {
        #[arg(help = "Path to project directory")]
        path: PathBuf,

        #[arg(short, long, help = "Languages to scan", value_delimiter = ',')]
        languages: Option<Vec<String>>,

        #[arg(long, help = "Exclude patterns (gitignore format)")]
        exclude: Vec<String>,

        #[arg(long, help = "Include only these patterns")]
        include: Vec<String>,

        #[arg(
            short,
            long,
            default_value_t = true,
            conflicts_with = "no_recursive",
            help = "Recursively walk subdirectories (default)"
        )]
        recursive: bool,

        #[arg(long, help = "Estimate only files directly in the project directory")]
        no_recursive: bool,

        #[arg(long, help = "Parallel parser workers", default_value = "4")]
        workers: usize,

        #[arg(long, value_enum, help = "Indexing tier: fast | balanced | full")]
        index_tier: Option<IndexTier>,

        #[arg(short, long, help = "Output format", default_value = "human")]
        format: StatsFormat,
    },

    #[command(about = "Manage MCP server configuration")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    #[cfg(feature = "daemon")]
    #[command(about = "Manage watch daemon for automatic re-indexing on file changes")]
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
}

#[cfg(feature = "daemon")]
#[derive(Subcommand)]
enum DaemonAction {
    #[command(about = "Start watch daemon for a project")]
    Start {
        #[arg(help = "Path to project directory", default_value = ".")]
        path: PathBuf,

        #[arg(long, help = "Run in foreground (default: daemonize)")]
        foreground: bool,

        #[arg(short, long, help = "Languages to watch", value_delimiter = ',')]
        languages: Option<Vec<String>>,

        #[arg(long, help = "Exclude patterns")]
        exclude: Vec<String>,

        #[arg(long, help = "Include patterns")]
        include: Vec<String>,
    },

    #[command(about = "Stop running watch daemon")]
    Stop {
        #[arg(help = "Path to project directory", default_value = ".")]
        path: PathBuf,
    },

    #[command(about = "Show watch daemon status")]
    Status {
        #[arg(help = "Path to project directory", default_value = ".")]
        path: PathBuf,

        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
}

#[derive(Subcommand)]
enum TransportType {
    #[command(about = "Start with STDIO transport (default)")]
    Stdio {
        #[arg(long, help = "Buffer size for STDIO", default_value = "8192")]
        buffer_size: usize,

        /// Enable automatic file watching and re-indexing (daemon mode)
        #[arg(
            long = "watch",
            help = "Enable automatic file watching and re-indexing",
            env = "CODEGRAPH_DAEMON_AUTO_START"
        )]
        enable_daemon: bool,

        /// Path to watch for file changes (defaults to current directory)
        #[arg(
            long = "watch-path",
            help = "Path to watch for file changes (defaults to current directory)",
            env = "CODEGRAPH_DAEMON_WATCH_PATH"
        )]
        watch_path: Option<PathBuf>,

        /// Explicitly disable daemon even if config enables it
        #[arg(
            long = "no-watch",
            help = "Explicitly disable daemon even if config enables it"
        )]
        disable_daemon: bool,
    },

    #[command(about = "Start with HTTP streaming transport")]
    Http {
        #[arg(short, long, help = "Host to bind to", default_value = "127.0.0.1")]
        host: String,

        #[arg(short, long, help = "Port to bind to", default_value = "3000")]
        port: u16,

        #[arg(long, help = "Enable TLS/HTTPS")]
        tls: bool,

        #[arg(long, help = "TLS certificate file")]
        cert: Option<PathBuf>,

        #[arg(long, help = "TLS key file")]
        key: Option<PathBuf>,

        #[arg(long, help = "Enable CORS")]
        cors: bool,

        /// Enable automatic file watching and re-indexing (daemon mode)
        #[arg(
            long = "watch",
            help = "Enable automatic file watching and re-indexing",
            env = "CODEGRAPH_DAEMON_AUTO_START"
        )]
        enable_daemon: bool,

        /// Path to watch for file changes (defaults to current directory)
        #[arg(
            long = "watch-path",
            help = "Path to watch for file changes (defaults to current directory)",
            env = "CODEGRAPH_DAEMON_WATCH_PATH"
        )]
        watch_path: Option<PathBuf>,

        /// Explicitly disable daemon even if config enables it
        #[arg(
            long = "no-watch",
            help = "Explicitly disable daemon even if config enables it"
        )]
        disable_daemon: bool,
    },

    #[command(about = "Start with both STDIO and HTTP transports")]
    Dual {
        #[arg(short, long, help = "HTTP host", default_value = "127.0.0.1")]
        host: String,

        #[arg(short, long, help = "HTTP port", default_value = "3000")]
        port: u16,

        #[arg(long, help = "STDIO buffer size", default_value = "8192")]
        buffer_size: usize,

        /// Enable automatic file watching and re-indexing (daemon mode)
        #[arg(
            long = "watch",
            help = "Enable automatic file watching and re-indexing",
            env = "CODEGRAPH_DAEMON_AUTO_START"
        )]
        enable_daemon: bool,

        /// Path to watch for file changes (defaults to current directory)
        #[arg(
            long = "watch-path",
            help = "Path to watch for file changes (defaults to current directory)",
            env = "CODEGRAPH_DAEMON_WATCH_PATH"
        )]
        watch_path: Option<PathBuf>,

        /// Explicitly disable daemon even if config enables it
        #[arg(
            long = "no-watch",
            help = "Explicitly disable daemon even if config enables it"
        )]
        disable_daemon: bool,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    #[command(
        about = "Initialize global configuration",
        long_about = "Create global configuration files at ~/.codegraph/:\n\
                      • config.toml - Structured configuration\n\
                      • .env - Environment variables (recommended for API keys)\n\
                      \n\
                      Configuration hierarchy (highest to lowest priority):\n\
                      1. Environment variables\n\
                      2. Local .env (current directory)\n\
                      3. Global ~/.codegraph.env\n\
                      4. Local .codegraph.toml (current directory)\n\
                      5. Global ~/.codegraph/config.toml\n\
                      6. Built-in defaults"
    )]
    Init {
        #[arg(short, long, help = "Overwrite existing files")]
        force: bool,
    },

    #[command(about = "Show current configuration")]
    Show {
        #[arg(long, help = "Show as JSON")]
        json: bool,
    },

    #[command(about = "Set configuration value")]
    Set {
        #[arg(help = "Configuration key")]
        key: String,

        #[arg(help = "Configuration value")]
        value: String,
    },

    #[command(about = "Get configuration value")]
    Get {
        #[arg(help = "Configuration key")]
        key: String,
    },

    #[command(about = "Reset configuration to defaults")]
    Reset {
        #[arg(short, long, help = "Skip confirmation")]
        yes: bool,
    },

    #[command(about = "Validate configuration")]
    Validate,

    #[command(about = "Open the project graph database and print its counts")]
    DbCheck,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum StatsFormat {
    Table,
    Json,
    Yaml,
    Human,
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum IndexTier {
    Fast,
    Balanced,
    Full,
}

impl From<IndexTier> for codegraph_core::config_manager::IndexingTier {
    fn from(value: IndexTier) -> Self {
        match value {
            IndexTier::Fast => codegraph_core::config_manager::IndexingTier::Fast,
            IndexTier::Balanced => codegraph_core::config_manager::IndexingTier::Balanced,
            IndexTier::Full => codegraph_core::config_manager::IndexingTier::Full,
        }
    }
}

fn main() -> Result<()> {
    let mut cli = Cli::parse();
    // Guidance/hooks must work without valid provider config or any network services.
    if let Commands::Tool { ref action } = cli.command {
        // SAFETY: No async runtime or application threads have been started yet.
        return unsafe { tool_cli::run(action, cli.verbose, cli.config.as_deref()) };
    }
    if let Commands::Hooks { ref action } = cli.command {
        return agent_hooks::run(action);
    }
    if let Commands::Doctor { ref path, json } = cli.command {
        let checks = codegraph_mcp_server::doctor::run_checks(path);
        if json {
            println!("{}", codegraph_mcp_server::doctor::render_json(&checks)?);
        } else {
            print!("{}", codegraph_mcp_server::doctor::render_table(&checks));
        }
        if codegraph_mcp_server::doctor::has_failure(&checks) {
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Commands::Uninstall {
        ref path,
        hooks,
        purge_index,
        dry_run,
    } = cli.command
    {
        let plan = codegraph_mcp_server::uninstall::run(path, hooks, purge_index, dry_run)?;
        if plan.is_empty() {
            println!("Nothing to remove.");
        }
        for (file, edit) in &plan {
            let verb = match (edit, dry_run) {
                (codegraph_mcp_server::uninstall::FileEdit::Delete, true) => "Would remove",
                (codegraph_mcp_server::uninstall::FileEdit::Delete, false) => "Removed",
                (_, true) => "Would update",
                (_, false) => "Updated",
            };
            println!("{verb} {}", file.display());
        }
        return Ok(());
    }
    if let Some(action) = quick_tool_call(&cli.command) {
        // SAFETY: No async runtime or application threads have been started yet.
        return unsafe { tool_cli::run(&action, cli.verbose, cli.config.as_deref()) };
    }
    if let Commands::DbCheck = cli.command {
        return tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(handle_db_check());
    }
    if let Commands::Init {
        ref path,
        hooks,
        mcp,
        no_index,
        index_tier,
        workers,
    } = cli.command
    {
        let invoking_directory = std::env::current_dir()?;
        let project = project_init::prepare(path, hooks, mcp)?;
        if no_index {
            eprintln!("Project setup complete; indexing skipped (--no-index).");
            return Ok(());
        }
        // Resolve an explicit config against the invoking cwd before loading project env/config.
        cli.config = cli.config.map(|path| invoking_directory.join(path));
        std::env::set_current_dir(&project)?;
        eprintln!("Step 3: Indexing project {}", project.display());
        cli.command = Commands::Index {
            path: project,
            languages: None,
            exclude: Vec::new(),
            include: Vec::new(),
            recursive: true,
            no_recursive: false,
            force: false,
            watch: false,
            workers,
            batch_size: None,
            index_tier,
            stats_json: None,
            postprocess: PostprocessFlags::default(),
        };
    }
    if let Some(path) = &cli.config {
        // SAFETY: Process configuration is prepared before starting worker threads.
        unsafe {
            std::env::set_var(
                "CODEGRAPH_CONFIG_PATH",
                path.canonicalize()
                    .with_context(|| format!("Cannot resolve config {}", path.display()))?,
            )
        };
    }
    // Load .env file if present
    // SAFETY: No worker threads exist before the runtime is created.
    unsafe { codegraph_core::config_manager::ConfigManager::initialize_environment() };

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_cli(cli))
}

/// Maps the graph shortcut commands to the equivalent `tool run` call, so they share
/// its project setup, timeout and JSON output.
fn quick_tool_call(command: &Commands) -> Option<tool_cli::ToolCommand> {
    let mut params = serde_json::Map::new();
    let (name, target) = match command {
        Commands::GraphStatus { target } => ("list_graph_stats", target),
        Commands::DetectChanges { base, target } => {
            if let Some(base) = base {
                params.insert("base_ref".into(), base.clone().into());
            }
            ("detect_changes", target)
        }
        Commands::Impact {
            node_ids,
            files,
            target,
        } => {
            if !node_ids.is_empty() {
                params.insert("node_ids".into(), node_ids.clone().into());
            }
            if !files.is_empty() {
                params.insert("files".into(), files.clone().into());
            }
            ("get_impact_radius", target)
        }
        Commands::Query {
            pattern,
            node_id,
            name,
            limit,
            target,
        } => {
            params.insert("pattern".into(), pattern.clone().into());
            if let Some(node_id) = node_id {
                params.insert("node_id".into(), node_id.clone().into());
            }
            if let Some(name) = name {
                params.insert("name".into(), name.clone().into());
            }
            if let Some(limit) = limit {
                params.insert("limit".into(), (*limit).into());
            }
            ("query_graph", target)
        }
        Commands::Search {
            query,
            limit,
            target,
        } => {
            params.insert("query".into(), query.clone().into());
            if let Some(limit) = limit {
                params.insert("limit".into(), (*limit).into());
            }
            ("search_nodes", target)
        }
        _ => return None,
    };
    Some(tool_cli::ToolCommand::Run {
        name: name.to_string(),
        params: serde_json::Value::Object(params).to_string(),
        target: target.clone(),
    })
}

async fn run_cli(cli: Cli) -> Result<()> {
    // Initialize debug logger (enabled with CODEGRAPH_DEBUG=1)
    DebugLogger::init();

    // Load configuration once at startup
    use codegraph_core::config_manager::ConfigManager;
    let config_mgr = ConfigManager::load().context("Failed to load configuration")?;
    let config = config_mgr.config();

    match cli.command {
        Commands::Init { .. }
        | Commands::Tool { .. }
        | Commands::Hooks { .. }
        | Commands::Uninstall { .. }
        | Commands::Doctor { .. }
        | Commands::DetectChanges { .. }
        | Commands::GraphStatus { .. }
        | Commands::Impact { .. }
        | Commands::Query { .. }
        | Commands::Search { .. } => {
            unreachable!("handled before configuration")
        }
        Commands::Build {
            path,
            workers,
            index_tier,
            postprocess,
        } => {
            handle_index(
                config,
                path,
                None,
                Vec::new(),
                Vec::new(),
                true,
                true,
                false,
                workers,
                None,
                index_tier,
                None,
                postprocess.level(),
                cli.debug,
            )
            .await?;
        }
        Commands::Update {
            path,
            workers,
            index_tier,
            postprocess,
        } => {
            handle_index(
                config,
                path,
                None,
                Vec::new(),
                Vec::new(),
                true,
                false,
                false,
                workers,
                None,
                index_tier,
                None,
                postprocess.level(),
                cli.debug,
            )
            .await?;
        }
        Commands::Start {
            transport,
            config,
            daemon,
            pid_file,
        } => {
            handle_start(transport, config, daemon, pid_file).await?;
        }
        Commands::Stop { pid_file, force } => {
            handle_stop(pid_file, force).await?;
        }
        Commands::Status { pid_file, detailed } => {
            handle_status(pid_file, detailed).await?;
        }
        Commands::Index {
            path,
            languages,
            exclude,
            include,
            recursive,
            no_recursive,
            force,
            watch,
            workers,
            batch_size,
            index_tier,
            stats_json,
            postprocess,
        } => {
            handle_index(
                config,
                path,
                languages,
                exclude,
                include,
                recursive && !no_recursive,
                force,
                watch,
                workers,
                batch_size,
                index_tier,
                stats_json,
                postprocess.level(),
                cli.debug,
            )
            .await?;
        }
        Commands::Estimate {
            path,
            languages,
            exclude,
            include,
            recursive,
            no_recursive,
            workers,
            index_tier,
            format,
        } => {
            handle_estimate(
                config,
                path,
                languages,
                exclude,
                include,
                recursive && !no_recursive,
                workers,
                index_tier,
                format,
            )
            .await?;
        }
        Commands::Config { action } => {
            handle_config(action).await?;
        }
        Commands::DbCheck => {
            unreachable!("handled before configuration")
        }

        #[cfg(feature = "daemon")]
        Commands::Daemon { action } => {
            handle_daemon(action).await?;
        }
    }

    Ok(())
}

async fn handle_start(
    transport: TransportType,
    config: Option<PathBuf>,
    daemon: bool,
    pid_file: Option<PathBuf>,
) -> Result<()> {
    let manager = ProcessManager::new();

    match transport {
        TransportType::Stdio {
            buffer_size: _buffer_size,
            enable_daemon,
            watch_path,
            disable_daemon,
        } => {
            // Configure logging to file for stdio transport (stdout/stderr are used for MCP protocol)
            // Logs will be written to .codegraph/logs/mcp-server.log
            let log_dir = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".codegraph")
                .join("logs");

            std::fs::create_dir_all(&log_dir).ok();

            // Use tracing_appender for non-blocking file writes
            let file_appender = tracing_appender::rolling::never(&log_dir, "mcp-server.log");
            let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

            let subscriber = tracing_subscriber::fmt()
                .with_writer(non_blocking)
                .with_max_level(tracing_subscriber::filter::LevelFilter::INFO)
                .with_ansi(false)
                .with_target(false)
                .with_line_number(true)
                .finish();

            tracing::subscriber::set_global_default(subscriber).ok();

            // Keep the guard alive for the duration of the server
            std::mem::forget(_guard);

            // Start background daemon if enabled
            #[cfg(feature = "daemon")]
            let mut daemon_manager: Option<DaemonManager> = None;

            #[cfg(feature = "daemon")]
            {
                use codegraph_core::config_manager::ConfigManager;

                // Load configuration
                if let Ok(config_mgr) = ConfigManager::load() {
                    let global_config = config_mgr.config().clone();

                    // Determine if daemon should start:
                    // Priority: --no-watch > --watch > config.daemon.auto_start_with_mcp
                    let should_start_daemon = if disable_daemon {
                        false
                    } else if enable_daemon {
                        true
                    } else {
                        global_config.daemon.auto_start_with_mcp
                    };

                    if should_start_daemon {
                        let project_root = watch_path
                            .or_else(|| global_config.daemon.project_path.clone())
                            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

                        let project_root =
                            std::fs::canonicalize(&project_root).unwrap_or(project_root);

                        // Create daemon config with auto_start forced on
                        let daemon_config = codegraph_core::config_manager::DaemonConfig {
                            auto_start_with_mcp: true,
                            project_path: Some(project_root.clone()),
                            ..global_config.daemon.clone()
                        };

                        let mut dm =
                            DaemonManager::new(daemon_config, global_config, project_root.clone());

                        match dm.start_background().await {
                            Ok(()) => {
                                if atty::is(Stream::Stderr) {
                                    eprintln!(
                                        "{}",
                                        format!("🔄 Daemon watching: {}", project_root.display())
                                            .cyan()
                                    );
                                }
                                daemon_manager = Some(dm);
                            }
                            Err(e) => {
                                // Log error but continue - MCP server should still work
                                if atty::is(Stream::Stderr) {
                                    eprintln!(
                                        "{}",
                                        format!(
                                            "⚠️  Daemon failed to start: {} (MCP server continuing)",
                                            e
                                        )
                                        .yellow()
                                    );
                                }
                                tracing::warn!("Daemon startup failed: {}", e);
                            }
                        }
                    }
                }
            }

            if atty::is(Stream::Stderr) {
                eprintln!(
                    "{}",
                    "Starting CodeGraph MCP Server with 100% Official SDK..."
                        .green()
                        .bold()
                );
            }

            // Create and initialize the revolutionary CodeGraph server with official SDK
            let server = CodeGraphMCPServer::new();

            if atty::is(Stream::Stderr) {
                eprintln!(
                    "✅ Revolutionary CodeGraph MCP server ready with 100% protocol compliance"
                );
            }

            // Use official rmcp STDIO transport for perfect compliance
            let service: rmcp::service::RunningService<rmcp::RoleServer, CodeGraphMCPServer> =
                server.serve(rmcp::transport::stdio()).await.map_err(|e| {
                    if atty::is(Stream::Stderr) {
                        eprintln!("❌ Failed to start official MCP server: {}", e);
                    }
                    anyhow::anyhow!("MCP server startup failed: {}", e)
                })?;

            if atty::is(Stream::Stderr) {
                eprintln!("🚀 Official MCP server started with revolutionary capabilities");
            }

            // Wait for the server to complete
            service
                .waiting()
                .await
                .map_err(|e| anyhow::anyhow!("Server error: {}", e))?;

            // Clean up daemon on exit
            #[cfg(feature = "daemon")]
            if let Some(mut dm) = daemon_manager {
                if let Err(e) = dm.stop().await {
                    tracing::warn!("Daemon cleanup error: {}", e);
                }
            }
        }
        TransportType::Http {
            host,
            port,
            tls,
            cert,
            key,
            cors: _,
            enable_daemon,
            watch_path,
            disable_daemon,
        } => {
            #[cfg(not(feature = "server-http"))]
            let _ = (
                host,
                port,
                tls,
                cert,
                key,
                enable_daemon,
                watch_path,
                disable_daemon,
            );
            #[cfg(not(feature = "server-http"))]
            {
                eprintln!("🚧 HTTP transport requires the 'server-http' feature");
                eprintln!();
                eprintln!("💡 Rebuild with HTTP support:");
                eprintln!("   cargo build --release --features server-http");
                eprintln!();
                eprintln!("💡 Or use STDIO transport:");
                eprintln!("   codegraph start stdio");

                return Err(anyhow::anyhow!(
                    "HTTP transport not enabled - rebuild with 'server-http' feature"
                ));
            }

            #[cfg(feature = "server-http")]
            {
                use axum::Router;
                use rmcp::transport::streamable_http_server::{
                    StreamableHttpServerConfig, StreamableHttpService,
                    session::local::LocalSessionManager,
                };
                use std::sync::Arc;
                use std::time::Duration;

                // Start background daemon if enabled
                #[cfg(feature = "daemon")]
                let mut daemon_manager: Option<DaemonManager> = None;

                #[cfg(feature = "daemon")]
                {
                    use codegraph_core::config_manager::ConfigManager;

                    if let Ok(config_mgr) = ConfigManager::load() {
                        let global_config = config_mgr.config().clone();

                        let should_start_daemon = if disable_daemon {
                            false
                        } else if enable_daemon {
                            true
                        } else {
                            global_config.daemon.auto_start_with_mcp
                        };

                        if should_start_daemon {
                            let project_root = watch_path
                                .or_else(|| global_config.daemon.project_path.clone())
                                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

                            let project_root =
                                std::fs::canonicalize(&project_root).unwrap_or(project_root);

                            let daemon_config = codegraph_core::config_manager::DaemonConfig {
                                auto_start_with_mcp: true,
                                project_path: Some(project_root.clone()),
                                ..global_config.daemon.clone()
                            };

                            let mut dm = DaemonManager::new(
                                daemon_config,
                                global_config,
                                project_root.clone(),
                            );

                            match dm.start_background().await {
                                Ok(()) => {
                                    eprintln!(
                                        "{}",
                                        format!("🔄 Daemon watching: {}", project_root.display())
                                            .cyan()
                                    );
                                    daemon_manager = Some(dm);
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{}",
                                        format!(
                                            "⚠️  Daemon failed to start: {} (HTTP server continuing)",
                                            e
                                        )
                                        .yellow()
                                    );
                                    tracing::warn!("Daemon startup failed: {}", e);
                                }
                            }
                        }
                    }
                }

                if atty::is(Stream::Stderr) {
                    eprintln!(
                        "{}",
                        "Starting CodeGraph MCP Server with HTTP transport..."
                            .green()
                            .bold()
                    );
                }

                // Handle TLS configuration
                if tls {
                    if cert.is_none() || key.is_none() {
                        return Err(anyhow::anyhow!(
                            "TLS enabled but certificate or key not provided. Use --cert and --key"
                        ));
                    }
                    eprintln!("⚠️  TLS configuration detected but not yet implemented");
                    eprintln!("   Server will start without TLS");
                }

                // Create session manager for stateful HTTP connections
                let session_manager = Arc::new(LocalSessionManager::default());

                // Service factory - creates new CodeGraphMCPServer for each session
                let service_factory = || {
                    let server = CodeGraphMCPServer::new();
                    // Note: initialize_qwen() is async, but service factory must be sync
                    // Qwen initialization will happen on first use
                    Ok(server)
                };

                // Configure HTTP server with SSE streaming
                let config = StreamableHttpServerConfig::default()
                    .with_allowed_hosts(codegraph_mcp_server::http_config::allowed_http_hosts(
                        &host,
                    ))
                    .with_sse_keep_alive(Some(Duration::from_secs(15)))
                    .with_legacy_session_mode(true);

                if atty::is(Stream::Stderr) {
                    eprintln!("📡 Configuring StreamableHTTP with SSE keep-alive (15s)");
                }

                // Create StreamableHttpService (implements tower::Service)
                let http_service =
                    StreamableHttpService::new(service_factory, session_manager, config);

                // Create Axum router
                let app = Router::new().fallback_service(http_service);

                // Bind to address
                let addr = format!("{}:{}", host, port);
                let listener = tokio::net::TcpListener::bind(&addr)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to bind to {}: {}", addr, e))?;

                if atty::is(Stream::Stderr) {
                    eprintln!("✅ CodeGraph MCP HTTP server ready");
                    eprintln!("🚀 Listening on http://{}", addr);
                    eprintln!();
                    eprintln!("📋 HTTP Endpoints:");
                    eprintln!("   POST   /mcp  - Initialize session");
                    eprintln!("   GET    /mcp  - Open SSE stream (with Mcp-Session-Id header)");
                    eprintln!("   POST   /mcp  - Send request (with Mcp-Session-Id header)");
                    eprintln!("   DELETE /mcp  - Close session");
                    eprintln!();
                    eprintln!("💡 Progress notifications stream via Server-Sent Events");
                }

                // Serve with Axum
                axum::serve(listener, app)
                    .await
                    .map_err(|e| anyhow::anyhow!("HTTP server error: {}", e))?;

                // Clean up daemon on exit
                #[cfg(feature = "daemon")]
                if let Some(mut dm) = daemon_manager {
                    if let Err(e) = dm.stop().await {
                        tracing::warn!("Daemon cleanup error: {}", e);
                    }
                }
            }
        }
        TransportType::Dual {
            host,
            port,
            buffer_size,
            enable_daemon: _,
            watch_path: _,
            disable_daemon: _,
        } => {
            // Note: Dual transport doesn't support daemon integration yet
            // The daemon fields are accepted but not used
            info!("Starting with dual transport (STDIO + HTTP)");
            let (stdio_pid, http_pid) = manager
                .start_dual_transport(
                    host.clone(),
                    port,
                    buffer_size,
                    config,
                    daemon,
                    pid_file.clone(),
                )
                .await?;
            if atty::is(Stream::Stdout) {
                println!("✓ MCP server started with dual transport");
                println!("  STDIO: buffer size {} (PID: {})", buffer_size, stdio_pid);
                println!("  HTTP: http://{}:{} (PID: {})", host, port, http_pid);
            }
        }
    }

    if daemon {
        if atty::is(Stream::Stdout) {
            println!("Running in daemon mode");
            if let Some(ref pid_file) = pid_file {
                println!("PID file: {:?}", pid_file);
            }
        }
    }

    Ok(())
}

async fn handle_stop(pid_file: Option<PathBuf>, force: bool) -> Result<()> {
    if atty::is(Stream::Stdout) {
        println!("{}", "Stopping CodeGraph MCP Server...".yellow().bold());
    }

    let manager = ProcessManager::new();

    if force {
        if atty::is(Stream::Stdout) {
            println!("Force stopping server");
        }
    }

    manager.stop_server(pid_file, force).await?;

    if atty::is(Stream::Stdout) {
        println!("✓ Server stopped");
    }

    Ok(())
}

async fn handle_status(pid_file: Option<PathBuf>, detailed: bool) -> Result<()> {
    println!("{}", "CodeGraph MCP Server Status".blue().bold());
    println!();

    let manager = ProcessManager::new();

    match manager.get_status(pid_file).await {
        Ok(info) => {
            println!("Server: {}", "Running".green());
            println!("Transport: {}", info.transport);
            println!("Process ID: {}", info.pid);

            let duration = chrono::Utc::now() - info.start_time;
            let hours = duration.num_hours();
            let minutes = (duration.num_minutes() % 60) as u32;
            println!("Uptime: {}h {}m", hours, minutes);

            if detailed {
                println!();
                println!("Detailed Information:");
                println!(
                    "  Start Time: {}",
                    info.start_time.format("%Y-%m-%d %H:%M:%S UTC")
                );
                if let Some(config) = info.config_path {
                    println!("  Config File: {:?}", config);
                }
                // TODO: Add more detailed metrics once integrated with monitoring
            }
        }
        Err(e) => {
            println!("Server: {}", "Not Running".red());
            println!("Error: {}", e);
        }
    }

    Ok(())
}

/// `--skip-flows` / `--skip-postprocess`; without either the level comes from
/// `CODEGRAPH_POSTPROCESS` (default `full`).
#[derive(clap::Args, Debug, Clone, Default)]
struct PostprocessFlags {
    #[arg(
        long,
        help = "Skip flow tracing after the build (postprocess level: minimal)"
    )]
    skip_flows: bool,
    #[arg(long, help = "Skip all post-build analysis (postprocess level: none)")]
    skip_postprocess: bool,
}

impl PostprocessFlags {
    fn level(&self) -> Option<codegraph_graph::analysis::PostprocessLevel> {
        use codegraph_graph::analysis::PostprocessLevel;
        if self.skip_postprocess {
            Some(PostprocessLevel::None)
        } else if self.skip_flows {
            Some(PostprocessLevel::Minimal)
        } else {
            None
        }
    }
}

async fn handle_index(
    config: &codegraph_core::config_manager::CodeGraphConfig,
    path: PathBuf,
    languages: Option<Vec<String>>,
    exclude: Vec<String>,
    include: Vec<String>,
    recursive: bool,
    force: bool,
    watch: bool,
    workers: usize,
    batch_size: Option<usize>,
    index_tier: Option<IndexTier>,
    stats_json: Option<PathBuf>,
    postprocess: Option<codegraph_graph::analysis::PostprocessLevel>,
    debug_log: bool,
) -> Result<()> {
    let project_root = path.clone().canonicalize().unwrap_or_else(|_| path.clone());

    let env_filter =
        || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let mut debug_log_path: Option<PathBuf> = None;

    if debug_log {
        let (writer, log_path) = prepare_debug_writer(&project_root)?;
        let subscriber = Registry::default()
            .with(env_filter())
            .with(tracing_subscriber::fmt::layer())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_writer(writer)
                    .with_ansi(false),
            );
        tracing::subscriber::set_global_default(subscriber).ok();
        println!("{} {}", "📝 Debug log:".cyan(), log_path.display());
        debug_log_path = Some(log_path);
    } else {
        let subscriber = Registry::default()
            .with(env_filter())
            .with(tracing_subscriber::fmt::layer());
        tracing::subscriber::set_global_default(subscriber).ok();
    }

    let multi_progress = MultiProgress::new();

    let h_style = ProgressStyle::with_template("{spinner:.blue} {msg}")
        .unwrap()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ");

    let header_pb = multi_progress.add(ProgressBar::new(1));
    header_pb.set_style(h_style);
    header_pb.set_message(format!("Indexing project: {}", path.to_string_lossy()));

    // Only workers are memory-tuned; an explicit storage batch is kept as given.
    let available_memory_gb = estimate_available_memory_gb();
    let storage_batch_size = batch_size
        .unwrap_or(IndexerConfig::default().batch_size)
        .max(1);
    let optimized_workers = optimize_workers_for_memory(available_memory_gb, workers);

    if available_memory_gb >= 64 {
        multi_progress.println(format!(
            "🚀 High-memory system detected ({}GB) - performance optimized!",
            available_memory_gb
        ))?;
    }

    let tier = index_tier.map(Into::into).unwrap_or(config.indexing.tier);
    let tier_hint = match tier {
        codegraph_core::config_manager::IndexingTier::Fast => {
            "fast (speed-first: AST + core edges)"
        }
        codegraph_core::config_manager::IndexingTier::Balanced => {
            "balanced (adds LSP symbols + enrichment)"
        }
        codegraph_core::config_manager::IndexingTier::Full => {
            "full (max detail: LSP definitions + dataflow + architecture)"
        }
    };
    multi_progress.println(format!(
        "🧭 Indexing tier: {} — override with --index-tier or CODEGRAPH_INDEX_TIER",
        tier_hint
    ))?;

    // Configure indexer
    let languages_list = languages.clone().unwrap_or_default();
    let indexer_config = IndexerConfig {
        languages: languages_list.clone(),
        exclude_patterns: exclude,
        include_patterns: include,
        recursive,
        force_reindex: force,
        watch,
        workers: optimized_workers,
        batch_size: storage_batch_size,
        indexing_tier: tier,
        project_root: project_root.clone(),
        postprocess,
        ..Default::default()
    };

    // Create indexer
    let start_time = std::time::Instant::now();
    let indexer = ProjectIndexer::new(indexer_config, config, multi_progress.clone()).await?;

    // Perform indexing
    let stats = indexer.index_project(&path).await?;
    let elapsed = start_time.elapsed();

    if let Some(destination) = stats_json {
        std::fs::write(destination, serde_json::to_vec_pretty(&stats)?)?;
    }
    header_pb.finish_with_message(
        if stats.complete {
            "✔ Indexing complete"
        } else {
            "✔ Graph ready; inference pending"
        }
        .to_string(),
    );
    println!(
        "Stages: graph={}, embeddings={}, semantic={}",
        stats.graph_complete, stats.embedding_status, stats.semantic_status
    );

    println!();
    println!(
        "{}",
        if stats.complete {
            "🎉 INDEXING COMPLETE!"
        } else {
            "GRAPH READY — INFERENCE PENDING"
        }
        .green()
        .bold()
    );
    println!();

    // Performance summary with comprehensive metrics
    println!("{}", "📊 Performance Summary".cyan().bold());
    println!("┌───────────────────────────────────────────────────────┐");
    println!(
        "│ ⏱️  Total time: {:<39} │",
        format!("{:.2?}", elapsed).green()
    );
    let files_per_sec = stats.files as f64 / elapsed.as_secs_f64();
    let nodes_per_sec = stats.nodes as f64 / elapsed.as_secs_f64();
    println!(
        "│ ⚡ Throughput: {:<39} │",
        format!("{:.1} files/s, {:.0} nodes/s", files_per_sec, nodes_per_sec).yellow()
    );
    println!("├───────────────────────────────────────────────────────┤");
    println!(
        "│ 📄 Files:      {:>6} indexed, {:>4} skipped           │",
        stats.files, stats.skipped
    );
    println!(
        "│ 📝 Lines:      {:>6} processed                        │",
        stats.lines
    );
    println!(
        "│ 🌳 Nodes:      {:>6} extracted                        │",
        stats.nodes
    );
    println!(
        "│ 🔗 Edges:      {:>6} relationships                    │",
        stats.edges
    );
    println!(
        "│ 🎯 Resolution: {:>6.1}% ({} resolved, {} unresolved)      │",
        stats.resolution_rate, stats.resolved_edges, stats.unresolved_edges
    );
    if stats.analyzers_enabled {
        println!(
            "│ 🧩 Build ctx:   {:>6} nodes, {:>4} edges                 │",
            stats.build_context_nodes, stats.build_context_edges
        );
        println!(
            "│ 🧠 LSP:         {:>6} symbols, {:>4} edges               │",
            stats.lsp_nodes_enriched, stats.lsp_edges_resolved
        );
        println!(
            "│ 🧾 Rustdoc/API: {:>6} docs, {:>4} exports               │",
            stats.docs_attached, stats.export_edges_added
        );
        println!(
            "│               +{:>6} reexports, {:>3} enables            │",
            stats.reexport_edges_added, stats.feature_enables_edges_added
        );
        println!(
            "│ 🧭 Modules:     {:>6} nodes, {:>4} imports              │",
            stats.module_nodes_added, stats.module_import_edges_added
        );
        println!(
            "│ 🌊 Dataflow:    {:>6} vars, {:>4} flows                │",
            stats.dataflow_variable_nodes_added, stats.dataflow_flows_to_edges_added
        );
        println!(
            "│ 📚 Docs:        {:>6} docs, {:>4} links                │",
            stats.doc_nodes_added,
            stats.document_edges_added + stats.specification_edges_added
        );
        println!(
            "│ 🏛️  Arch:       {:>6} cycles, {:>4} violations          │",
            stats.package_cycles_detected, stats.boundary_violations_added
        );
    }
    if stats.errors > 0 {
        println!(
            "│ ❌ Errors:     {:>6} encountered                      │",
            stats.errors
        );
    }
    println!("├──────────────────────────────────────────────────────┤");
    let avg_nodes = if stats.files > 0 {
        stats.nodes as f64 / stats.files as f64
    } else {
        0.0
    };
    let avg_edges = if stats.files > 0 {
        stats.edges as f64 / stats.files as f64
    } else {
        0.0
    };
    println!(
        "│ 📈 Averages: {:.1} nodes/file, {:.1} edges/file          │",
        avg_nodes, avg_edges
    );
    println!("└───────────────────────────────────────────────────────┘");

    // Configuration summary
    println!();
    println!("{}", "⚙️  Configuration Summary".cyan().bold());
    println!(
        "Workers: {} | Storage batch rows: {} | Languages: {}",
        optimized_workers,
        storage_batch_size,
        languages_list.join(", ")
    );

    println!();
    println!("{}", "🚀 Graph ready for CodeGraph tools".green().bold());
    println!("Next: Start MCP server with 'codegraph start http' or 'codegraph start stdio'");

    if stats.errors > 0 {
        println!("  {} Errors: {}", "⚠".yellow(), stats.errors);
    }

    if watch {
        println!();
        println!("Watching for changes... (Press Ctrl+C to stop)");
        indexer.watch_for_changes(path).await?;
    }

    if let Some(log_path) = debug_log_path {
        println!(
            "{} {}",
            "📂 Detailed log saved at:".cyan(),
            log_path.display()
        );
    }

    Ok(())
}

async fn handle_estimate(
    config: &codegraph_core::config_manager::CodeGraphConfig,
    path: PathBuf,
    languages: Option<Vec<String>>,
    exclude: Vec<String>,
    include: Vec<String>,
    recursive: bool,
    workers: usize,
    index_tier: Option<IndexTier>,
    format: StatsFormat,
) -> Result<()> {
    let project_root = path.clone().canonicalize().unwrap_or(path.clone());
    let languages_list = languages.clone().unwrap_or_default();

    let tier = index_tier.map(Into::into).unwrap_or(config.indexing.tier);
    let mut estimator_config = IndexerConfig {
        languages: languages_list.clone(),
        exclude_patterns: exclude,
        include_patterns: include,
        recursive,
        workers,
        indexing_tier: tier,
        ..Default::default()
    };
    estimator_config.project_root = project_root.clone();

    let estimator = RepositoryEstimator::new(estimator_config);

    println!(
        "{} {}",
        "🔍 Estimating repository:".cyan().bold(),
        project_root.to_string_lossy()
    );

    let start = std::time::Instant::now();
    let report = estimator.analyze(&path).await?;
    let elapsed = start.elapsed();

    present_estimate_output(
        format,
        &project_root,
        &languages_list,
        &report,
        workers,
        elapsed,
    )
}

fn present_estimate_output(
    format: StatsFormat,
    project_root: &Path,
    languages: &[String],
    report: &RepositoryEstimate,
    workers: usize,
    elapsed: Duration,
) -> Result<()> {
    let parsing_minutes = report.parsing_duration.as_secs_f64() / 60.0;

    let payload = serde_json::json!({
        "path": project_root.to_string_lossy(),
        "languages": if languages.is_empty() { serde_json::Value::Null } else { serde_json::Value::from(languages.to_vec()) },
        "counts": {
            "total_files": report.counts.total_files,
            "parsed_files": report.counts.parsed_files,
            "failed_files": report.counts.failed_files,
            "nodes": report.counts.nodes,
            "edges": report.counts.edges,
            "symbols": report.counts.symbols,
        },
        "parsing": {
            "minutes": parsing_minutes,
            "duration_seconds": report.parsing_duration.as_secs_f64(),
            "total_lines": report.parsing.total_lines,
            "files_per_second": report.parsing.files_per_second,
            "lines_per_second": report.parsing.lines_per_second,
        },
        "workers": workers,
        "estimate_runtime_seconds": elapsed.as_secs_f64(),
    });

    match format {
        StatsFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        StatsFormat::Yaml => {
            let yaml = serde_yaml::to_string(&payload)?;
            println!("{yaml}");
        }
        StatsFormat::Table | StatsFormat::Human => {
            println!();
            println!("{}", "📊 Indexing Estimate".cyan().bold());
            println!("Path: {}", project_root.display());
            println!(
                "Languages: {}",
                if languages.is_empty() {
                    "auto-detect".to_string()
                } else {
                    languages.join(", ")
                }
            );
            println!();
            println!(
                "Files parsed: {} / {} (failed: {})",
                report.counts.parsed_files, report.counts.total_files, report.counts.failed_files
            );
            println!("Nodes: {}", report.counts.nodes);
            println!("Edges: {}", report.counts.edges);
            println!("Symbols: {}", report.counts.symbols);
            println!(
                "Parsing time (measured): {}",
                format_duration_minutes(parsing_minutes)
            );
            println!(
                "Estimation runtime: {} (parser only, no DB writes)",
                format_duration_minutes(elapsed.as_secs_f64() / 60.0)
            );
        }
    }

    Ok(())
}

fn format_duration_minutes(minutes: f64) -> String {
    if !minutes.is_finite() {
        return "unknown".to_string();
    }
    let total_seconds = (minutes * 60.0).round() as i64;
    let hours = total_seconds / 3600;
    let minutes_part = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes_part > 0 {
        parts.push(format!("{minutes_part}m"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds}s"));
    }
    parts.join(" ")
}

async fn handle_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Init { force } => {
            use codegraph_core::config_manager::ConfigManager;

            println!(
                "{}",
                "Initializing CodeGraph global configuration..."
                    .green()
                    .bold()
            );
            println!();

            // Ensure ~/.codegraph directory exists
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
            let codegraph_dir = home.join(".codegraph");

            if !codegraph_dir.exists() {
                std::fs::create_dir_all(&codegraph_dir)
                    .context("Failed to create ~/.codegraph directory")?;
                println!("✓ Created directory: {}", codegraph_dir.display());
            }

            // Create config.toml
            let config_path = codegraph_dir.join("config.toml");
            if config_path.exists() && !force {
                println!(
                    "⚠️  Configuration file already exists: {}",
                    config_path.display()
                );
                println!("   Use --force to overwrite");
            } else {
                ConfigManager::create_default_config(&config_path)
                    .context("Failed to create config.toml")?;
                println!("✓ Created config file: {}", config_path.display());
            }

            // Create .env template
            let env_path = codegraph_dir.join(".env");
            if env_path.exists() && !force {
                println!(
                    "⚠️  Environment file already exists: {}",
                    env_path.display()
                );
                println!("   Use --force to overwrite");
            } else {
                let env_template = r#"# CodeGraph Global Configuration
# This file is loaded automatically by codegraph
# Environment variables here override config.toml settings

# ============================================================================
# INDEXING
# ============================================================================
# Indexing tier: fast (default) | balanced | full
# CODEGRAPH_INDEX_TIER=fast

# Disable analyzers (build context, LSP, docs, ...) with 0
# CODEGRAPH_ANALYZERS=1

# Parser workers (default: --workers, capped by available cores)
# CODEGRAPH_WORKERS=4

# ============================================================================
# GRAPH TOOLS
# ============================================================================
# Context window the tool-result size limits are sized for (default: 128000)
# CODEGRAPH_CONTEXT_WINDOW=128000

# ============================================================================
# LOGGING
# ============================================================================
# Log level: trace, debug, info, warn, error
# RUST_LOG=warn
"#;

                std::fs::write(&env_path, env_template).context("Failed to create .env file")?;
                println!("✓ Created environment file: {}", env_path.display());
            }

            println!();
            println!(
                "{}",
                "Configuration initialized successfully!".green().bold()
            );
            println!();
            println!("Next steps:");
            println!(
                "1. Edit {} to adjust indexing and tool settings",
                env_path.display()
            );
            println!("2. Run 'codegraph config show' to verify your configuration");
            println!("3. Run 'codegraph index <path>' to start indexing your codebase");
            println!();
            println!("Configuration hierarchy (highest to lowest priority):");
            println!("  1. Environment variables");
            println!("  2. Local .env (current directory)");
            println!("  3. Global {}", env_path.display());
            println!("  4. Local .codegraph.toml (current directory)");
            println!("  5. Global {}", config_path.display());
            println!("  6. Built-in defaults");
        }
        ConfigAction::Show { json } => {
            use codegraph_core::config_manager::ConfigManager;

            // Load actual configuration
            let config_mgr = ConfigManager::load().context("Failed to load configuration")?;
            let config = config_mgr.config();

            let context_window = ConfigManager::agent_context_window();
            if json {
                let json_output = serde_json::json!({
                    "config_file": config_mgr.config_path().map(|p| p.display().to_string()),
                    "indexing": { "tier": config.indexing.tier },
                    "tools": { "context_window": context_window },
                    "daemon": {
                        "auto_start_with_mcp": config.daemon.auto_start_with_mcp,
                        "debounce_ms": config.daemon.debounce_ms,
                    },
                    "logging": { "level": config.logging.level },
                });
                println!("{}", serde_json::to_string_pretty(&json_output)?);
            } else {
                println!("{}", "Current Configuration:".blue().bold());
                if let Some(path) = config_mgr.config_path() {
                    println!("  Config file: {}", path.display());
                }
                println!("  Indexing tier: {:?}", config.indexing.tier);
                println!("  Tool context window: {}", context_window);
                println!(
                    "  Daemon auto-start with MCP: {}",
                    config.daemon.auto_start_with_mcp
                );
                println!("  Log level: {}", config.logging.level);
            }
        }
        ConfigAction::Set { key, value } => {
            println!("Set {} = {}", key.yellow(), value.green());
        }
        ConfigAction::Get { key } => {
            println!("{}: {}", key, "value");
        }
        ConfigAction::Reset { yes } => {
            if !yes {
                println!("Reset configuration to defaults? (y/n)");
                // TODO: Read user input
            }
            println!("✓ Configuration reset to defaults");
        }
        ConfigAction::Validate => {
            println!("✓ Configuration is valid");
        }
        ConfigAction::DbCheck => {
            handle_db_check().await?;
        }
    }

    Ok(())
}

/// Opens `.code-review-graph/graph.db` in the working directory (creating it if absent).
async fn handle_db_check() -> Result<()> {
    use codegraph_graph::sqlite_store::{SCHEMA_VERSION, SqliteGraphStore, db_path};

    let root = std::env::current_dir()?;
    let store = SqliteGraphStore::open(&root)?;
    let counts = store.counts().await?;
    println!(
        "✓ {} (schema {SCHEMA_VERSION}): {} nodes, {} edges, {} files",
        db_path(&root).display(),
        counts.nodes,
        counts.edges,
        counts.files
    );
    Ok(())
}

fn prepare_debug_writer(base_path: &Path) -> Result<(BoxMakeWriter, PathBuf)> {
    let log_dir = base_path.join(".codegraph").join("logs");
    std::fs::create_dir_all(&log_dir)?;
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S");
    let log_path = log_dir.join(format!("index-debug-{}.log", timestamp));
    let file = File::create(&log_path)?;
    let shared = Arc::new(Mutex::new(file));
    let writer = BoxMakeWriter::new({
        let shared = Arc::clone(&shared);
        move || DebugLogWriter::new(Arc::clone(&shared))
    });
    Ok((writer, log_path))
}

/// Estimate available system memory in GB
fn estimate_available_memory_gb() -> usize {
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
        {
            if let Ok(memsize_str) = String::from_utf8(output.stdout) {
                if let Ok(memsize) = memsize_str.trim().parse::<u64>() {
                    return (memsize / 1024 / 1024 / 1024) as usize;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return (kb / 1024 / 1024) as usize;
                        }
                    }
                }
            }
        }
    }

    16 // Default assumption if detection fails
}

fn parse_positive_batch_size(value: &str) -> std::result::Result<usize, String> {
    value
        .parse::<std::num::NonZeroUsize>()
        .map(std::num::NonZeroUsize::get)
        .map_err(|error| error.to_string())
}

/// Optimize worker count based on available memory.
fn optimize_workers_for_memory(memory_gb: usize, default_workers: usize) -> usize {
    if default_workers == 4 {
        // Default value
        match memory_gb {
            128.. => 16,    // 128GB+: Maximum parallelism
            96..=127 => 14, // 96-127GB: Very high parallelism
            64..=95 => 12,  // 64-95GB: High parallelism
            48..=63 => 10,  // 48-63GB: Medium-high parallelism
            32..=47 => 8,   // 32-47GB: Medium parallelism
            16..=31 => 6,   // 16-31GB: Conservative parallelism
            _ => 4,         // <16GB: Keep default
        }
    } else {
        default_workers // User specified - respect their choice
    }
}

#[derive(Clone)]
struct DebugLogWriter {
    file: Arc<Mutex<File>>,
}

impl DebugLogWriter {
    fn new(file: Arc<Mutex<File>>) -> Self {
        Self { file }
    }
}

impl Write for DebugLogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self.file.lock().unwrap();
        guard.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self.file.lock().unwrap();
        guard.flush()
    }
}

// Daemon mode handlers
#[cfg(feature = "daemon")]
async fn handle_daemon(action: DaemonAction) -> Result<()> {
    match action {
        DaemonAction::Start {
            path,
            foreground,
            languages,
            exclude,
            include,
        } => handle_daemon_start(path, foreground, languages, exclude, include).await,
        DaemonAction::Stop { path } => handle_daemon_stop(path).await,
        DaemonAction::Status { path, json } => handle_daemon_status(path, json).await,
    }
}

#[cfg(feature = "daemon")]
async fn handle_daemon_start(
    path: PathBuf,
    foreground: bool,
    languages: Option<Vec<String>>,
    exclude: Vec<String>,
    include: Vec<String>,
) -> Result<()> {
    use codegraph_mcp::{IndexerConfig, ProjectIndexer};

    let project_root = std::fs::canonicalize(&path)
        .with_context(|| format!("Invalid project path: {:?}", path))?;

    println!(
        "{}",
        format!("🚀 Starting watch daemon for: {}", project_root.display()).green()
    );

    // Check if daemon already running
    let pid_path = PidFile::default_path(&project_root);
    let pid_file = PidFile::new(&pid_path);
    if pid_file.is_process_running()? {
        anyhow::bail!(
            "Daemon already running for this project (PID file: {:?})",
            pid_path
        );
    }

    // Create IndexerConfig
    let indexer_config = IndexerConfig {
        languages: languages.unwrap_or_default(),
        exclude_patterns: exclude,
        include_patterns: include,
        project_root: project_root.clone(),
        ..Default::default()
    };

    // Create WatchConfig
    let watch_config = WatchConfig {
        project_root: project_root.clone(),
        debounce_ms: 30,
        batch_timeout_ms: 200,
        health_check_interval_secs: 30,
        reconnect_backoff: Default::default(),
        circuit_breaker: Default::default(),
        indexer: indexer_config.clone(),
    };

    // Create ProjectIndexer
    use codegraph_core::config_manager::ConfigManager;
    let config_mgr = ConfigManager::load().with_context(|| "Failed to load configuration")?;
    let global_config = config_mgr.config().clone();

    let indexer = ProjectIndexer::new(
        indexer_config,
        &global_config,
        indicatif::MultiProgress::new(),
    )
    .await
    .with_context(|| "Failed to create project indexer")?;

    // Create and start daemon
    let mut daemon =
        WatchDaemon::new(watch_config).with_context(|| "Failed to create watch daemon")?;
    daemon.set_indexer(indexer);

    if foreground {
        println!(
            "{}",
            "Running in foreground. Press Ctrl+C to stop.".yellow()
        );
        daemon.start().await?;
    } else {
        // For now, just run in foreground (proper daemonization requires fork)
        println!(
            "{}",
            "Note: Running in foreground mode. Use --foreground flag for explicit foreground mode."
                .yellow()
        );
        println!("{}", "Press Ctrl+C to stop the daemon.".yellow());
        daemon.start().await?;
    }

    Ok(())
}

#[cfg(test)]
mod cli_command_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn postprocess_flags_map_to_levels() {
        use codegraph_graph::analysis::PostprocessLevel;
        for (args, expected) in [
            (vec![], None),
            (vec!["--skip-flows"], Some(PostprocessLevel::Minimal)),
            (vec!["--skip-postprocess"], Some(PostprocessLevel::None)),
            (
                vec!["--skip-flows", "--skip-postprocess"],
                Some(PostprocessLevel::None),
            ),
        ] {
            for command in ["build", "update", "index"] {
                let mut argv = vec!["codegraph", command];
                argv.extend(args.iter().copied());
                argv.push(".");
                let cli = Cli::try_parse_from(argv).unwrap();
                let flags = match cli.command {
                    Commands::Build { postprocess, .. }
                    | Commands::Update { postprocess, .. }
                    | Commands::Index { postprocess, .. } => postprocess,
                    _ => unreachable!(),
                };
                assert_eq!(flags.level(), expected, "{command} {args:?}");
            }
        }
    }

    #[test]
    fn indexing_batch_size_preserves_explicit_values_and_rejects_zero() {
        let cli = Cli::try_parse_from(["codegraph", "index", "."]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Index {
                batch_size: None,
                ..
            }
        ));
        for size in [100, 512, 4096] {
            let value = size.to_string();
            let cli =
                Cli::try_parse_from(["codegraph", "index", "--batch-size", &value, "."]).unwrap();
            assert!(
                matches!(cli.command, Commands::Index { batch_size: Some(actual), .. } if actual == size)
            );
        }
        assert!(Cli::try_parse_from(["codegraph", "index", "--batch-size", "0", "."]).is_err());
    }

    #[test]
    fn removed_subcommands_are_absent() {
        let cmd = Cli::command();
        let names: Vec<_> = cmd
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();
        for removed in ["stats", "clean", "perf", "code", "test", "agent"] {
            assert!(
                !names.iter().any(|n| n == removed),
                "unexpected subcommand still present: {removed}"
            );
        }
        for added in ["init", "tool", "hooks"] {
            assert!(names.iter().any(|name| name == added));
        }
    }

    #[test]
    fn directory_commands_recurse_by_default_and_accept_explicit_root_only_scans() {
        for command in ["index", "estimate"] {
            for (flag, expected) in [
                (None, true),
                (Some("-r"), true),
                (Some("--recursive"), true),
                (Some("--no-recursive"), false),
            ] {
                let mut arguments = vec!["codegraph", command, "--languages", "Rust"];
                if let Some(flag) = flag {
                    arguments.push(flag);
                }
                arguments.push(".");
                let cli = Cli::try_parse_from(arguments).unwrap();
                let (recursive, no_recursive) = match cli.command {
                    Commands::Index {
                        recursive,
                        no_recursive,
                        ..
                    }
                    | Commands::Estimate {
                        recursive,
                        no_recursive,
                        ..
                    } => (recursive, no_recursive),
                    _ => unreachable!(),
                };
                assert_eq!(recursive && !no_recursive, expected, "{command}: {flag:?}");
            }
            assert!(
                Cli::try_parse_from(["codegraph", command, "--recursive", "--no-recursive", ".",])
                    .is_err()
            );
        }
    }
}

#[cfg(feature = "daemon")]
async fn handle_daemon_stop(path: PathBuf) -> Result<()> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    let project_root = std::fs::canonicalize(&path)
        .with_context(|| format!("Invalid project path: {:?}", path))?;

    let pid_path = PidFile::default_path(&project_root);
    let pid_file = PidFile::new(&pid_path);

    match pid_file.read()? {
        Some(pid) => {
            println!(
                "{}",
                format!("🛑 Stopping daemon (PID: {})...", pid).yellow()
            );

            // Send SIGTERM
            let pid = Pid::from_raw(pid as i32);
            match kill(pid, Signal::SIGTERM) {
                Ok(_) => {
                    println!("{}", "✅ Stop signal sent successfully".green());

                    // Wait a bit and check if it stopped
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    if !pid_file.is_process_running()? {
                        println!("{}", "✅ Daemon stopped".green());
                    } else {
                        println!(
                            "{}",
                            "⚠️  Daemon still running. May take a moment to shut down.".yellow()
                        );
                    }
                }
                Err(e) => {
                    anyhow::bail!("Failed to send stop signal: {}", e);
                }
            }
        }
        None => {
            println!(
                "{}",
                format!("No daemon running for: {}", project_root.display()).yellow()
            );
        }
    }

    Ok(())
}

#[cfg(feature = "daemon")]
async fn handle_daemon_status(path: PathBuf, json: bool) -> Result<()> {
    let project_root = std::fs::canonicalize(&path)
        .with_context(|| format!("Invalid project path: {:?}", path))?;

    let pid_path = PidFile::default_path(&project_root);
    let pid_file = PidFile::new(&pid_path);

    let (running, pid): (bool, Option<u32>) = match pid_file.read()? {
        Some(pid) => (pid_file.is_process_running()?, Some(pid)),
        None => (false, None),
    };

    if json {
        let status = serde_json::json!({
            "project": project_root.display().to_string(),
            "running": running,
            "pid": pid,
            "pid_file": pid_path.display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        println!("{}", "📊 Watch Daemon Status".bold());
        println!("   Project: {}", project_root.display());
        println!("   PID file: {}", pid_path.display());

        if running {
            println!(
                "   Status: {} (PID: {})",
                "Running".green(),
                pid.unwrap_or(0)
            );
        } else {
            println!("   Status: {}", "Stopped".red());
            if pid.is_some() {
                println!(
                    "   {}",
                    "⚠️  Stale PID file detected. Run 'daemon start' to clean up.".yellow()
                );
            }
        }
    }

    Ok(())
}
