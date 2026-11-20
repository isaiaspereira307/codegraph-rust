use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{info, warn};

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Config file not found: {0}")]
    NotFound(String),

    #[error("Failed to read config: {0}")]
    ReadError(String),

    #[error("Failed to parse config: {0}")]
    ParseError(String),

    #[error("Invalid configuration: {0}")]
    ValidationError(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),
}

/// Main configuration for CodeGraph
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodeGraphConfig {
    /// Performance and resource settings
    #[serde(default)]
    pub performance: PerformanceConfig,

    /// Indexing configuration
    #[serde(default)]
    pub indexing: IndexingConfig,

    /// Logging configuration
    #[serde(default)]
    pub logging: LoggingConfig,

    /// Daemon configuration for automatic file watching
    #[serde(default)]
    pub daemon: DaemonConfig,
}

/// Performance and resource configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Number of worker threads
    #[serde(default = "default_num_threads")]
    pub num_threads: usize,

    /// Cache size in MB
    #[serde(default = "default_cache_size_mb")]
    pub cache_size_mb: usize,

    /// Enable GPU acceleration
    #[serde(default)]
    pub enable_gpu: bool,

    /// Maximum concurrent requests
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_requests: usize,
}

/// Indexing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    /// Indexing tier: fast | balanced | full
    #[serde(default)]
    pub tier: IndexingTier,
}

impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            tier: IndexingTier::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IndexingTier {
    Fast,
    Balanced,
    Full,
}

impl Default for IndexingTier {
    fn default() -> Self {
        IndexingTier::Fast
    }
}

impl std::str::FromStr for IndexingTier {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "fast" => Ok(IndexingTier::Fast),
            "balanced" => Ok(IndexingTier::Balanced),
            "full" => Ok(IndexingTier::Full),
            other => Err(format!("Invalid indexing tier: {}", other)),
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            num_threads: default_num_threads(),
            cache_size_mb: default_cache_size_mb(),
            enable_gpu: false, // Conservative default
            max_concurrent_requests: default_max_concurrent(),
        }
    }
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level: "trace", "debug", "info", "warn", "error"
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log format: "pretty", "json", "compact"
    #[serde(default = "default_log_format")]
    pub format: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
        }
    }
}

/// Daemon configuration for automatic file watching and re-indexing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Enable automatic daemon startup with MCP server
    #[serde(default)]
    pub auto_start_with_mcp: bool,

    /// Project path to watch (defaults to current directory)
    #[serde(default)]
    pub project_path: Option<PathBuf>,

    /// Debounce duration for file changes (ms)
    #[serde(default = "default_daemon_debounce_ms")]
    pub debounce_ms: u64,

    /// Batch timeout for collecting changes (ms)
    #[serde(default = "default_daemon_batch_timeout_ms")]
    pub batch_timeout_ms: u64,

    /// Health check interval (seconds)
    #[serde(default = "default_daemon_health_check_interval")]
    pub health_check_interval_secs: u64,

    /// Languages to watch (empty = all detected)
    #[serde(default)]
    pub languages: Vec<String>,

    /// Exclude patterns (gitignore format)
    #[serde(default = "default_daemon_exclude_patterns")]
    pub exclude_patterns: Vec<String>,

    /// Include patterns (gitignore format)
    #[serde(default)]
    pub include_patterns: Vec<String>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            auto_start_with_mcp: false, // Opt-in by default
            project_path: None,
            debounce_ms: default_daemon_debounce_ms(),
            batch_timeout_ms: default_daemon_batch_timeout_ms(),
            health_check_interval_secs: default_daemon_health_check_interval(),
            languages: vec![],
            exclude_patterns: default_daemon_exclude_patterns(),
            include_patterns: vec![],
        }
    }
}

// Default value functions
/// Context window assumed when none is configured.
pub const DEFAULT_CONTEXT_WINDOW: usize = 128_000;

/// `[llm] context_window` as written in a config file. Invalid TOML, a missing key or a
/// value of the wrong type yields `None`. The rest of `[llm]` is ignored.
fn context_window_from_toml(content: &str) -> Option<usize> {
    content
        .parse::<toml::Table>()
        .ok()?
        .get("llm")?
        .get("context_window")?
        .as_integer()
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v > 0)
}

fn default_num_threads() -> usize {
    num_cpus::get()
}
fn default_cache_size_mb() -> usize {
    512
}
fn default_max_concurrent() -> usize {
    4
}
fn default_log_level() -> String {
    "warn".to_string()
} // Clean TUI output during indexing
fn default_log_format() -> String {
    "pretty".to_string()
}

// Daemon default functions
fn default_daemon_debounce_ms() -> u64 {
    30
}
fn default_daemon_batch_timeout_ms() -> u64 {
    200
}
fn default_daemon_health_check_interval() -> u64 {
    30
}
fn default_daemon_exclude_patterns() -> Vec<String> {
    vec![
        "**/node_modules/**".to_string(),
        "**/target/**".to_string(),
        "**/.git/**".to_string(),
        "**/build/**".to_string(),
        "**/.codegraph/**".to_string(),
        "**/dist/**".to_string(),
        "**/__pycache__/**".to_string(),
    ]
}

/// Configuration manager with smart defaults and auto-detection
pub struct ConfigManager {
    config: CodeGraphConfig,
    config_path: Option<PathBuf>,
}

impl ConfigManager {
    /// Load configuration with the following precedence:
    /// 1. Environment variables (initialize dotenv before starting workers)
    /// 2. Config file (.codegraph.toml)
    /// 3. Sensible defaults
    pub fn load() -> Result<Self, ConfigError> {
        info!("🔧 Loading CodeGraph configuration...");

        // Try to find and load config file
        let (config, config_path) = Self::load_config_file()?;

        // Override with environment variables
        let config = Self::apply_env_overrides(config);

        // Validate configuration
        Self::validate_config(&config)?;

        info!("✅ Configuration loaded successfully");
        if let Some(ref path) = config_path {
            info!("   📄 Config file: {}", path.display());
        } else {
            info!("   📄 Config file: NONE (using defaults)");
        }

        Ok(Self {
            config,
            config_path,
        })
    }

    /// Load project `.env` or user `.codegraph.env` before starting workers.
    /// `load()` itself only reads configuration and never mutates the environment.
    ///
    /// # Safety
    /// Call only from a single-threaded process entry point, before other threads
    /// can access the environment (including through foreign libraries).
    pub unsafe fn initialize_environment() {
        // Try current directory first
        if Path::new(".env").exists() {
            if let Err(e) = dotenv::from_filename(".env") {
                warn!("Failed to load .env file: {}", e);
            } else {
                info!("📋 Loaded .env file from current directory");
            }
            return;
        }

        // Try home directory
        if let Some(home) = dirs::home_dir() {
            let home_env = home.join(".codegraph.env");
            if home_env.exists() {
                if let Err(e) = dotenv::from_path(&home_env) {
                    warn!("Failed to load .codegraph.env: {}", e);
                } else {
                    info!("📋 Loaded .codegraph.env from home directory");
                }
            }
        }
    }

    /// Find and load config file
    /// Search order:
    /// 1. CODEGRAPH_CONFIG_PATH (explicit CLI/environment selection)
    /// 2. ./.codegraph.toml (current directory)
    /// 3. ~/.codegraph/config.toml (user config)
    /// 4. Use defaults
    /// Locate the configuration file: an explicit CLI/environment selection, then the
    /// project file in the working directory, then the user file.
    fn config_file_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("CODEGRAPH_CONFIG_PATH") {
            return Some(PathBuf::from(path));
        }
        let local_config = Path::new(".codegraph.toml");
        if local_config.exists() {
            return Some(local_config.to_path_buf());
        }
        dirs::home_dir()
            .map(|home| home.join(".codegraph").join("config.toml"))
            .filter(|user_config| user_config.exists())
    }

    fn load_config_file() -> Result<(CodeGraphConfig, Option<PathBuf>), ConfigError> {
        match Self::config_file_path() {
            Some(path) => {
                let config = Self::read_toml_file(&path)?;
                Ok((config, Some(path)))
            }
            None => {
                info!("📋 No config file found, using defaults");
                Ok((CodeGraphConfig::default(), None))
            }
        }
    }

    /// Context window the agent and its tool-result limits are sized for:
    /// `CODEGRAPH_CONTEXT_WINDOW`, then `CODEGRAPH_LLM_CONTEXT_WINDOW`, then
    /// `[llm] context_window` in the config file, then the default.
    pub fn agent_context_window() -> usize {
        std::env::var("CODEGRAPH_CONTEXT_WINDOW")
            .or_else(|_| std::env::var("CODEGRAPH_LLM_CONTEXT_WINDOW"))
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .or_else(|| {
                let path = Self::config_file_path()?;
                context_window_from_toml(&std::fs::read_to_string(path).ok()?)
            })
            .unwrap_or(DEFAULT_CONTEXT_WINDOW)
    }

    fn read_toml_file(path: &Path) -> Result<CodeGraphConfig, ConfigError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ConfigError::ReadError(e.to_string()))?;

        let config: CodeGraphConfig =
            toml::from_str(&content).map_err(|e| ConfigError::ParseError(e.to_string()))?;

        Ok(config)
    }

    /// Apply environment variable overrides
    fn apply_env_overrides(mut config: CodeGraphConfig) -> CodeGraphConfig {
        // Indexing configuration
        if let Ok(tier) = std::env::var("CODEGRAPH_INDEX_TIER") {
            match tier.parse::<IndexingTier>() {
                Ok(parsed) => config.indexing.tier = parsed,
                Err(err) => warn!("Invalid CODEGRAPH_INDEX_TIER: {}", err),
            }
        }

        // Logging
        if let Ok(level) = std::env::var("RUST_LOG") {
            config.logging.level = level;
        }

        // Daemon configuration
        if let Ok(auto_start) = std::env::var("CODEGRAPH_DAEMON_AUTO_START") {
            config.daemon.auto_start_with_mcp =
                auto_start.to_lowercase() == "true" || auto_start == "1";
        }
        if let Ok(path) = std::env::var("CODEGRAPH_DAEMON_WATCH_PATH") {
            config.daemon.project_path = Some(PathBuf::from(path));
        }
        if let Ok(debounce) = std::env::var("CODEGRAPH_DAEMON_DEBOUNCE_MS") {
            if let Ok(ms) = debounce.parse() {
                config.daemon.debounce_ms = ms;
            }
        }
        if let Ok(batch) = std::env::var("CODEGRAPH_DAEMON_BATCH_TIMEOUT_MS") {
            if let Ok(ms) = batch.parse() {
                config.daemon.batch_timeout_ms = ms;
            }
        }

        config
    }

    /// Validate configuration
    fn validate_config(config: &CodeGraphConfig) -> Result<(), ConfigError> {
        // Validate log level
        match config.logging.level.as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => {}
            other => {
                return Err(ConfigError::ValidationError(format!(
                    "Invalid log level: {}. Must be one of: trace, debug, info, warn, error",
                    other
                )));
            }
        }

        Ok(())
    }

    /// Get the loaded configuration
    pub fn config(&self) -> &CodeGraphConfig {
        &self.config
    }

    /// Get the path to the config file that was loaded, if any
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Create a default config file
    pub fn create_default_config(path: &Path) -> Result<(), ConfigError> {
        let config = CodeGraphConfig::default();
        let toml_str =
            toml::to_string_pretty(&config).map_err(|e| ConfigError::ParseError(e.to_string()))?;

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::ReadError(e.to_string()))?;
        }

        std::fs::write(path, toml_str).map_err(|e| ConfigError::ReadError(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CodeGraphConfig::default();
        assert_eq!(config.indexing.tier, IndexingTier::Fast);
    }

    #[test]
    fn test_indexing_tier_env_override() {
        if !test_env::run(
            concat!(module_path!(), "::test_indexing_tier_env_override"),
            &[("CODEGRAPH_INDEX_TIER", Some("balanced"))],
        ) {
            return;
        }

        let config = ConfigManager::apply_env_overrides(CodeGraphConfig::default());
        assert_eq!(config.indexing.tier, IndexingTier::Balanced);
    }

    #[test]
    fn context_window_from_toml_reads_only_a_valid_llm_key() {
        let toml = "[embedding]\nprovider = \"ollama\"\n\n[llm]\ncontext_window = 200000\n";
        assert_eq!(context_window_from_toml(toml), Some(200_000));
        assert_eq!(context_window_from_toml("[llm]\nprovider = \"x\"\n"), None);
        assert_eq!(
            context_window_from_toml("[llm]\ncontext_window = \"big\"\n"),
            None
        );
        assert_eq!(
            context_window_from_toml("[llm]\ncontext_window = 0\n"),
            None
        );
        assert_eq!(context_window_from_toml("not = [valid"), None);
    }

    #[test]
    fn legacy_inference_sections_are_ignored() {
        let config: CodeGraphConfig = toml::from_str(
            "[embedding]\nprovider = \"jina\"\n[llm]\nenabled = true\n[rerank]\nprovider = \"jina\"\n",
        )
        .unwrap();
        assert!(ConfigManager::validate_config(&config).is_ok());
    }

    #[test]
    fn test_config_validation() {
        let config = CodeGraphConfig::default();
        assert!(ConfigManager::validate_config(&config).is_ok());

        let mut bad_config = config.clone();
        bad_config.logging.level = "loud".to_string();
        assert!(ConfigManager::validate_config(&bad_config).is_err());
    }
}

#[cfg(test)]
mod test_env {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/support/env.rs"
    ));
}
