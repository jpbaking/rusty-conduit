//! Configuration loading and validation.
//!
//! Sources are merged with strict precedence:
//! **CLI flags > `RUSTY_CONDUIT_*` environment variables > optional `--config` TOML file > defaults**.
//!
//! CLI flags and environment variables are both handled by `clap` (each flag declares its
//! `env` fallback, and `clap` itself prefers the flag over the variable). The resulting
//! [`CliInput`] is then layered over an optional [`FileConfig`] and the built-in defaults
//! by [`Config::merge`], and checked by [`Config::validate`].

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::parser::ValueSource;
use clap::{ArgAction, ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};
use serde::Deserialize;

use crate::filter::{FilterRule, HostFilter};
use crate::http::ErrorPageTemplate;
use crate::logging::{LogFormat, LogLevel};
use crate::mitm::{MitmOptions, UpstreamTlsVerification};
use crate::net::KeepaliveSettings;
use crate::routing::RoutingInput;
use crate::server::ServerOptions;
use rusty_conduit_engine::cli_support::file::{
    merge_logging, merge_mitm, FileValueError, LoggingCliValues, MitmCliValues,
};

/// Default bind address.
pub const DEFAULT_BIND: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// Default HTTP/HTTPS proxy port.
pub const DEFAULT_HTTP_PORT: u16 = 3128;
/// Default SOCKS4/4a/5 proxy port.
pub const DEFAULT_SOCKS_PORT: u16 = 1080;
/// Default in-memory TTL for generated MITM leaf certificates.
pub const DEFAULT_MITM_CERT_TTL: Duration = Duration::from_secs(60 * 60);
/// Smallest accepted MITM leaf certificate TTL.
pub const MIN_MITM_CERT_TTL: Duration = Duration::from_secs(10);
/// Default cap on concurrent client connections (`0` = unlimited).
pub const DEFAULT_MAX_CONNECTIONS: usize = 4096;
/// Largest accepted TCP keepalive idle time / probe interval (Linux caps both at 32767s).
pub const MAX_TCP_KEEPALIVE_DURATION: Duration = Duration::from_secs(32_767);
/// Largest accepted TCP keepalive probe count (Linux caps `TCP_KEEPCNT` at 127).
pub const MAX_TCP_KEEPALIVE_RETRIES: u32 = 127;
/// Default cap on cached MITM leaf certificates (`0` = unlimited).
pub const DEFAULT_MITM_CERT_CACHE_MAX_ENTRIES: usize = 10_240;

/// The version `--version` and the startup banner report.
///
/// Release builds set `CONDUIT_RELEASE_VERSION` at compile time to the tag without its
/// leading `v` (pre-release suffixes such as `-beta.1` included); any other build reports
/// the `Cargo.toml` version with a `-dev` suffix.
pub fn app_version() -> &'static str {
    rusty_conduit_engine::app_version!()
}

/// Errors raised while loading or validating configuration. Every variant is `FATAL`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config file {path}: {message}")]
    ParseFile { path: PathBuf, message: String },
    #[error("invalid value for `{key}` in config file: {message}")]
    InvalidFileValue { key: &'static str, message: String },
    #[error("both http_port and socks_port are 0: at least one listener must be enabled")]
    NoListeners,
    #[error(
        "ambiguous MITM CA source: --mitm-ca-p12 cannot be combined with --mitm-ca-cert or --mitm-ca-key"
    )]
    AmbiguousCaSource,
    #[error("incomplete PEM MITM CA: --mitm-ca-cert and --mitm-ca-key must be provided together")]
    IncompletePemCa,
    #[error(
        "conflicting passphrase sources: use only one of --mitm-ca-passphrase and --mitm-ca-passphrase-file"
    )]
    ConflictingPassphrase,
    #[error("--mitm-cert-ttl must be at least 10s (got {})", humantime::format_duration(*.0))]
    CertTtlTooShort(Duration),
    #[error("failed to read passphrase file {path}: {source}")]
    ReadPassphraseFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TCP keepalive setting: {0}")]
    InvalidKeepalive(String),
    #[error("invalid --mitm-insecure-host rule {rule:?}: {message}")]
    InvalidMitmInsecureHost { rule: String, message: String },
    #[error("invalid --mitm-bypass-host rule {rule:?}: {message}")]
    InvalidMitmBypassHost { rule: String, message: String },
}

/// Subcommands. Without one, the proxy runs.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Validate the configuration (flags, environment, and --config file) and the files it
    /// names (MITM CA, log paths), then exit 0/1
    Validate {
        /// Check the settings only; skip the files they name (for checking a config away from
        /// the host it runs on)
        #[arg(long)]
        settings_only: bool,
    },
}

/// Raw command-line / environment input (`clap` derive with `env` fallbacks).
#[derive(Debug, Clone, Default, Parser)]
#[command(
    name = "rusty-conduit",
    version = app_version(),
    about = "Lightweight HTTP/HTTPS and SOCKS4/4a/5 proxy server with optional HTTPS MITM inspection",
    long_about = "Lightweight HTTP/HTTPS and SOCKS4/4a/5 proxy server with optional HTTPS MITM inspection.\n\n\
                  Precedence: CLI flags > RUSTY_CONDUIT_* environment variables > --config TOML file > defaults."
)]
pub struct CliArgs {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Optional TOML configuration file path
    #[arg(long, value_name = "PATH", env = "RUSTY_CONDUIT_CONFIG", global = true)]
    pub config: Option<PathBuf>,

    /// IPv4 or IPv6 address to bind listeners to [default: 127.0.0.1]
    #[arg(long, value_name = "IP", env = "RUSTY_CONDUIT_BIND")]
    pub bind: Option<IpAddr>,

    /// HTTP/HTTPS proxy port, 0 to disable [default: 3128]
    #[arg(long, value_name = "PORT", env = "RUSTY_CONDUIT_HTTP_PORT")]
    pub http_port: Option<u16>,

    /// SOCKS4/4a/5 proxy port, 0 to disable [default: 1080]
    #[arg(long, value_name = "PORT", env = "RUSTY_CONDUIT_SOCKS_PORT")]
    pub socks_port: Option<u16>,

    /// Maximum concurrent client connections; excess connections are closed immediately, 0 = unlimited [default: 4096]
    #[arg(long, value_name = "N", env = "RUSTY_CONDUIT_MAX_CONNECTIONS")]
    pub max_connections: Option<usize>,

    /// Idle time before TCP keepalive probes start on client and upstream sockets, 0 = disabled [default: 60s]
    #[arg(
        long,
        value_name = "DURATION",
        env = "RUSTY_CONDUIT_TCP_KEEPALIVE",
        value_parser = humantime::parse_duration
    )]
    pub tcp_keepalive: Option<Duration>,

    /// Time between unanswered TCP keepalive probes [default: 10s]
    #[arg(
        long,
        value_name = "DURATION",
        env = "RUSTY_CONDUIT_TCP_KEEPALIVE_INTERVAL",
        value_parser = humantime::parse_duration
    )]
    pub tcp_keepalive_interval: Option<Duration>,

    /// Unanswered TCP keepalive probes before the connection is closed [default: 5]
    #[arg(long, value_name = "N", env = "RUSTY_CONDUIT_TCP_KEEPALIVE_RETRIES")]
    pub tcp_keepalive_retries: Option<u32>,

    /// Allowlist of destinations (IP, CIDR, FQDN, or *.domain); repeatable or comma-separated.
    /// When combined with --block, serves as exceptions to the blocklist.
    #[arg(
        long,
        value_name = "RULE",
        value_delimiter = ',',
        env = "RUSTY_CONDUIT_ALLOW"
    )]
    pub allow: Vec<FilterRule>,

    /// Blocklist of destinations (IP, CIDR, FQDN, or *.domain); repeatable or comma-separated.
    /// Combine with --allow to carve out exceptions.
    #[arg(
        long,
        value_name = "RULE",
        value_delimiter = ',',
        env = "RUSTY_CONDUIT_BLOCK"
    )]
    pub block: Vec<FilterRule>,

    /// Path to PEM-encoded CA certificate (enables MITM together with --mitm-ca-key)
    #[arg(long, value_name = "PATH", env = "RUSTY_CONDUIT_MITM_CA_CERT")]
    pub mitm_ca_cert: Option<PathBuf>,

    /// Path to PEM-encoded CA private key
    #[arg(long, value_name = "PATH", env = "RUSTY_CONDUIT_MITM_CA_KEY")]
    pub mitm_ca_key: Option<PathBuf>,

    /// Path to PKCS#12 (.p12/.pfx) CA bundle (enables MITM)
    #[arg(long, value_name = "PATH", env = "RUSTY_CONDUIT_MITM_CA_P12")]
    pub mitm_ca_p12: Option<PathBuf>,

    /// Passphrase for the PKCS#12 bundle or encrypted PEM key (prefer the env var or a passphrase file)
    #[arg(
        long,
        value_name = "STR",
        env = "RUSTY_CONDUIT_MITM_CA_PASSPHRASE",
        hide_env_values = true
    )]
    pub mitm_ca_passphrase: Option<String>,

    /// File containing the CA passphrase (trailing newline is trimmed)
    #[arg(
        long,
        value_name = "PATH",
        env = "RUSTY_CONDUIT_MITM_CA_PASSPHRASE_FILE"
    )]
    pub mitm_ca_passphrase_file: Option<PathBuf>,

    /// In-memory TTL for generated MITM leaf certs, e.g. 30m, 1h, 3600s [default: 1h]
    #[arg(
        long,
        value_name = "DURATION",
        env = "RUSTY_CONDUIT_MITM_CERT_TTL",
        value_parser = humantime::parse_duration
    )]
    pub mitm_cert_ttl: Option<Duration>,

    /// Maximum cached MITM leaf certificates; expired then oldest entries are evicted, 0 = unlimited [default: 10240]
    #[arg(
        long,
        value_name = "N",
        env = "RUSTY_CONDUIT_MITM_CERT_CACHE_MAX_ENTRIES"
    )]
    pub mitm_cert_cache_max_entries: Option<usize>,

    /// Accept invalid/self-signed upstream TLS certificates under MITM for matching destinations
    /// (IP, CIDR, FQDN, or *.domain); repeatable or comma-separated
    #[arg(
        long = "mitm-insecure-host",
        value_name = "RULE",
        value_delimiter = ',',
        env = "RUSTY_CONDUIT_MITM_INSECURE_HOST"
    )]
    pub mitm_insecure_host: Vec<String>,

    /// Exclude matching destinations (IP, CIDR, FQDN, or *.domain) from MITM interception: they
    /// get an opaque CONNECT tunnel instead; repeatable or comma-separated
    #[arg(
        long = "mitm-bypass-host",
        value_name = "RULE",
        value_delimiter = ',',
        env = "RUSTY_CONDUIT_MITM_BYPASS_HOST"
    )]
    pub mitm_bypass_host: Vec<String>,

    /// UNSAFE blind trust: skip upstream TLS certificate verification for ALL MITM targets
    #[arg(
        long = "mitm-insecure",
        env = "RUSTY_CONDUIT_MITM_INSECURE",
        action = ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub mitm_insecure: bool,

    /// Reject HTTP/HTTPS traffic over SOCKS4/4a/5 (works with or without MITM)
    #[arg(
        long,
        env = "RUSTY_CONDUIT_SOCKS_BLOCK_WEB",
        action = ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub socks_block_web: bool,

    /// HTML template file for proxy-generated HTTP error pages (placeholders: {{status}},
    /// {{reason}}, {{message}}, {{target}}) [default: built-in page]
    #[arg(long, value_name = "PATH", env = "RUSTY_CONDUIT_ERROR_PAGE")]
    pub error_page: Option<PathBuf>,

    /// Log level: QUIET, FATAL, ERROR, INFO, DEBUG, VERBOSE (case-insensitive) [default: INFO]
    #[arg(long, value_name = "LEVEL", env = "RUSTY_CONDUIT_LOG_LEVEL")]
    pub log_level: Option<LogLevel>,

    /// Access/SOCKS log format: text (logfmt) or json (NDJSON) [default: text]
    #[arg(long, value_name = "FMT", env = "RUSTY_CONDUIT_LOG_FORMAT")]
    pub log_format: Option<LogFormat>,

    /// Optional file path for HTTP/HTTPS access logs
    #[arg(long, value_name = "PATH", env = "RUSTY_CONDUIT_HTTP_ACCESS_LOG")]
    pub http_access_log: Option<PathBuf>,

    /// Optional file path for SOCKS4/4a/5 connection logs
    #[arg(long, value_name = "PATH", env = "RUSTY_CONDUIT_SOCKS_LOG")]
    pub socks_log: Option<PathBuf>,
}

/// Parsed CLI/env input plus the provenance details that `clap` alone does not expose.
#[derive(Debug, Clone, Default)]
pub struct CliInput {
    pub args: CliArgs,
    /// `--mitm-ca-passphrase` came from an actual command-line argument (not the env var).
    pub passphrase_from_cli: bool,
    /// `--socks-block-web` was explicitly provided (CLI or env).
    pub socks_block_web_explicit: bool,
    /// Resolved CLI/env value for `--socks-block-web` when `socks_block_web_explicit` is true.
    pub socks_block_web_value: bool,
}

impl CliInput {
    /// Parse the process arguments and environment; prints help/version or usage errors and exits.
    pub fn parse() -> Self {
        let matches = CliArgs::command().get_matches();
        Self::from_matches(&matches).unwrap_or_else(|e| e.exit())
    }

    /// Parse an explicit argument list (the environment is still consulted for `env` fallbacks).
    pub fn try_parse_from<I, T>(args: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = CliArgs::command().try_get_matches_from(args)?;
        Self::from_matches(&matches)
    }

    fn from_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        let args = CliArgs::from_arg_matches(matches)?;
        let passphrase_from_cli =
            matches.value_source("mitm_ca_passphrase") == Some(ValueSource::CommandLine);
        let socks_block_web_explicit = matches!(
            matches.value_source("socks_block_web"),
            Some(ValueSource::CommandLine | ValueSource::EnvVariable)
        );
        let socks_block_web_value = args.socks_block_web;
        Ok(Self {
            args,
            passphrase_from_cli,
            socks_block_web_explicit,
            socks_block_web_value,
        })
    }
}

/// Top-level TOML configuration file layout.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub bind: Option<IpAddr>,
    pub http_port: Option<u16>,
    pub socks_port: Option<u16>,
    pub max_connections: Option<usize>,
    /// Human-readable duration such as `60s` or `2m`; `"0"` disables keepalive.
    pub tcp_keepalive: Option<String>,
    /// Human-readable duration such as `10s`.
    pub tcp_keepalive_interval: Option<String>,
    pub tcp_keepalive_retries: Option<u32>,
    pub allow: Option<Vec<String>>,
    pub block: Option<Vec<String>>,
    /// Reject HTTP/HTTPS traffic over SOCKS4/4a/5 (works with or without MITM).
    pub socks_block_web: Option<bool>,
    /// HTML template file for proxy-generated HTTP error pages.
    pub error_page: Option<PathBuf>,
    #[serde(default)]
    pub mitm: FileMitmConfig,
    #[serde(default)]
    pub logging: FileLoggingConfig,
}

/// The `[mitm]` and `[logging]` tables, shared with the other rusty-conduit CLIs.
pub use rusty_conduit_engine::cli_support::file::{FileLoggingConfig, FileMitmConfig};

impl FileConfig {
    /// Parse TOML text.
    pub fn from_toml_str(text: &str, path: &Path) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::ParseFile {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
    }

    /// Read and parse a TOML file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&text, path)
    }
}

/// Where the MITM CA comes from (only present when MITM mode is ON).
pub use crate::mitm::CaSource;

/// Fully merged configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct Config {
    pub config_path: Option<PathBuf>,
    pub bind: IpAddr,
    pub http_port: u16,
    pub socks_port: u16,
    /// Maximum concurrent client connections (`0` = unlimited).
    pub max_connections: usize,
    /// Idle time before TCP keepalive probes start (`0` = keepalive disabled).
    pub tcp_keepalive: Duration,
    /// Time between unanswered TCP keepalive probes.
    pub tcp_keepalive_interval: Duration,
    /// Unanswered TCP keepalive probes before the OS closes the connection.
    pub tcp_keepalive_retries: u32,
    /// Destination allowlist rules (exceptions to `block` when both are set).
    pub allow: Vec<FilterRule>,
    /// Destination blocklist rules (combine with `allow` to carve out exceptions).
    pub block: Vec<FilterRule>,
    pub mitm_ca_cert: Option<PathBuf>,
    pub mitm_ca_key: Option<PathBuf>,
    pub mitm_ca_p12: Option<PathBuf>,
    pub mitm_ca_passphrase: Option<String>,
    pub mitm_ca_passphrase_file: Option<PathBuf>,
    pub mitm_cert_ttl: Duration,
    /// Maximum cached MITM leaf certificates (`0` = unlimited).
    pub mitm_cert_cache_max_entries: usize,
    /// Upstream TLS verification is skipped for MITM targets matching these rules
    /// (`FilterRule` syntax; checked by [`Config::validate`]).
    pub mitm_insecure_hosts: Vec<String>,
    /// MITM interception is skipped (opaque `CONNECT` tunnel) for targets matching these rules
    /// (`FilterRule` syntax; checked by [`Config::validate`]).
    pub mitm_bypass_hosts: Vec<String>,
    /// Blind trust: upstream TLS verification is skipped for every MITM target.
    pub mitm_insecure: bool,
    pub socks_block_web: bool,
    /// HTML template for proxy-generated HTTP error pages (`None` = built-in page).
    pub error_page: Option<PathBuf>,
    pub log_level: LogLevel,
    pub log_format: LogFormat,
    pub http_access_log: Option<PathBuf>,
    pub socks_log: Option<PathBuf>,
    /// The passphrase was supplied as a CLI argument (triggers an `INFO` notice).
    pub passphrase_from_cli: bool,
    /// Notices produced while merging the config file (for example a deprecated key).
    pub file_notices: Vec<String>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("config_path", &self.config_path)
            .field("bind", &self.bind)
            .field("http_port", &self.http_port)
            .field("socks_port", &self.socks_port)
            .field("max_connections", &self.max_connections)
            .field("tcp_keepalive", &self.tcp_keepalive)
            .field("tcp_keepalive_interval", &self.tcp_keepalive_interval)
            .field("tcp_keepalive_retries", &self.tcp_keepalive_retries)
            .field("allow", &self.allow)
            .field("block", &self.block)
            .field("mitm_ca_cert", &self.mitm_ca_cert)
            .field("mitm_ca_key", &self.mitm_ca_key)
            .field("mitm_ca_p12", &self.mitm_ca_p12)
            .field(
                "mitm_ca_passphrase",
                &self.mitm_ca_passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("mitm_ca_passphrase_file", &self.mitm_ca_passphrase_file)
            .field("mitm_cert_ttl", &self.mitm_cert_ttl)
            .field(
                "mitm_cert_cache_max_entries",
                &self.mitm_cert_cache_max_entries,
            )
            .field("mitm_insecure_hosts", &self.mitm_insecure_hosts)
            .field("mitm_bypass_hosts", &self.mitm_bypass_hosts)
            .field("mitm_insecure", &self.mitm_insecure)
            .field("socks_block_web", &self.socks_block_web)
            .field("error_page", &self.error_page)
            .field("log_level", &self.log_level)
            .field("log_format", &self.log_format)
            .field("http_access_log", &self.http_access_log)
            .field("socks_log", &self.socks_log)
            .field("passphrase_from_cli", &self.passphrase_from_cli)
            .field("file_notices", &self.file_notices)
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_path: None,
            bind: DEFAULT_BIND,
            http_port: DEFAULT_HTTP_PORT,
            socks_port: DEFAULT_SOCKS_PORT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            tcp_keepalive: KeepaliveSettings::DEFAULT.idle,
            tcp_keepalive_interval: KeepaliveSettings::DEFAULT.interval,
            tcp_keepalive_retries: KeepaliveSettings::DEFAULT.retries,
            allow: Vec::new(),
            block: Vec::new(),
            mitm_ca_cert: None,
            mitm_ca_key: None,
            mitm_ca_p12: None,
            mitm_ca_passphrase: None,
            mitm_ca_passphrase_file: None,
            mitm_cert_ttl: DEFAULT_MITM_CERT_TTL,
            mitm_cert_cache_max_entries: DEFAULT_MITM_CERT_CACHE_MAX_ENTRIES,
            mitm_insecure_hosts: Vec::new(),
            mitm_bypass_hosts: Vec::new(),
            mitm_insecure: false,
            socks_block_web: false,
            error_page: None,
            log_level: LogLevel::Info,
            log_format: LogFormat::Text,
            http_access_log: None,
            socks_log: None,
            passphrase_from_cli: false,
            file_notices: Vec::new(),
        }
    }
}

impl Config {
    /// Load the TOML file named by `--config` (if any) and merge it under the CLI/env input.
    /// Does not validate; call [`Config::validate`] afterwards.
    pub fn from_cli(cli: CliInput) -> Result<Self, ConfigError> {
        let file = match &cli.args.config {
            Some(path) => Some(FileConfig::load(path)?),
            None => None,
        };
        Self::merge(cli, file)
    }

    /// Merge CLI/env input over an optional file config over defaults. Does not validate.
    pub fn merge(cli: CliInput, file: Option<FileConfig>) -> Result<Self, ConfigError> {
        let file = file.unwrap_or_default();
        let d = Config::default();
        let a = cli.args;

        let file_value = |e: FileValueError| ConfigError::InvalidFileValue {
            key: e.key,
            message: e.message,
        };
        let mitm = merge_mitm(
            MitmCliValues {
                ca_cert: a.mitm_ca_cert,
                ca_key: a.mitm_ca_key,
                ca_p12: a.mitm_ca_p12,
                ca_passphrase: a.mitm_ca_passphrase,
                ca_passphrase_file: a.mitm_ca_passphrase_file,
                cert_ttl: a.mitm_cert_ttl,
                cert_cache_max_entries: a.mitm_cert_cache_max_entries,
                insecure_hosts: a.mitm_insecure_host,
                bypass_hosts: a.mitm_bypass_host,
                insecure: a.mitm_insecure,
            },
            Some(file.mitm),
        )
        .map_err(file_value)?;
        let file_keepalive = file_duration("tcp_keepalive", file.tcp_keepalive.as_deref())?;
        let file_keepalive_interval = file_duration(
            "tcp_keepalive_interval",
            file.tcp_keepalive_interval.as_deref(),
        )?;
        // After the keepalive keys, so file errors are reported in the former order.
        let logging = merge_logging(
            LoggingCliValues {
                level: a.log_level,
                format: a.log_format,
                http_access_log: a.http_access_log,
                socks_log: a.socks_log,
            },
            Some(file.logging),
        )
        .map_err(file_value)?;
        let socks_block_web = if cli.socks_block_web_explicit {
            cli.socks_block_web_value
        } else {
            file.socks_block_web.unwrap_or(d.socks_block_web)
        };
        let file_allow = file_filter_rules("allow", file.allow)?;
        let file_block = file_filter_rules("block", file.block)?;
        let allow = if a.allow.is_empty() {
            file_allow
        } else {
            a.allow
        };
        let block = if a.block.is_empty() {
            file_block
        } else {
            a.block
        };
        Ok(Config {
            config_path: a.config,
            bind: a.bind.or(file.bind).unwrap_or(d.bind),
            http_port: a.http_port.or(file.http_port).unwrap_or(d.http_port),
            socks_port: a.socks_port.or(file.socks_port).unwrap_or(d.socks_port),
            max_connections: a
                .max_connections
                .or(file.max_connections)
                .unwrap_or(d.max_connections),
            tcp_keepalive: a
                .tcp_keepalive
                .or(file_keepalive)
                .unwrap_or(d.tcp_keepalive),
            tcp_keepalive_interval: a
                .tcp_keepalive_interval
                .or(file_keepalive_interval)
                .unwrap_or(d.tcp_keepalive_interval),
            tcp_keepalive_retries: a
                .tcp_keepalive_retries
                .or(file.tcp_keepalive_retries)
                .unwrap_or(d.tcp_keepalive_retries),
            allow,
            block,
            mitm_ca_cert: mitm.ca_cert,
            mitm_ca_key: mitm.ca_key,
            mitm_ca_p12: mitm.ca_p12,
            mitm_ca_passphrase: mitm.ca_passphrase,
            mitm_ca_passphrase_file: mitm.ca_passphrase_file,
            mitm_cert_ttl: mitm.cert_ttl,
            mitm_cert_cache_max_entries: mitm.cert_cache_max_entries,
            mitm_insecure_hosts: mitm.insecure_hosts,
            mitm_bypass_hosts: mitm.bypass_hosts,
            mitm_insecure: mitm.insecure,
            socks_block_web,
            error_page: a.error_page.or(file.error_page),
            log_level: logging.level,
            log_format: logging.format,
            http_access_log: logging.http_access_log,
            socks_log: logging.socks_log,
            passphrase_from_cli: cli.passphrase_from_cli,
            file_notices: mitm.notices,
        })
    }

    /// Apply the `FATAL` startup validation rules.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.http_port == 0 && self.socks_port == 0 {
            return Err(ConfigError::NoListeners);
        }
        if self.mitm_ca_p12.is_some() && (self.mitm_ca_cert.is_some() || self.mitm_ca_key.is_some())
        {
            return Err(ConfigError::AmbiguousCaSource);
        }
        if self.mitm_ca_cert.is_some() != self.mitm_ca_key.is_some() {
            return Err(ConfigError::IncompletePemCa);
        }
        if self.mitm_ca_passphrase.is_some() && self.mitm_ca_passphrase_file.is_some() {
            return Err(ConfigError::ConflictingPassphrase);
        }
        if self.mitm_cert_ttl < MIN_MITM_CERT_TTL {
            return Err(ConfigError::CertTtlTooShort(self.mitm_cert_ttl));
        }
        self.mitm_insecure_rules()?;
        self.mitm_bypass_rules()?;
        if !self.tcp_keepalive.is_zero() {
            let secs = Duration::from_secs(1)..=MAX_TCP_KEEPALIVE_DURATION;
            let fmt = |d: Duration| humantime::format_duration(d).to_string();
            if !secs.contains(&self.tcp_keepalive) {
                return Err(ConfigError::InvalidKeepalive(format!(
                    "--tcp-keepalive must be 0 (disabled) or between 1s and {}s (got {})",
                    MAX_TCP_KEEPALIVE_DURATION.as_secs(),
                    fmt(self.tcp_keepalive)
                )));
            }
            if !secs.contains(&self.tcp_keepalive_interval) {
                return Err(ConfigError::InvalidKeepalive(format!(
                    "--tcp-keepalive-interval must be between 1s and {}s (got {})",
                    MAX_TCP_KEEPALIVE_DURATION.as_secs(),
                    fmt(self.tcp_keepalive_interval)
                )));
            }
            if !(1..=MAX_TCP_KEEPALIVE_RETRIES).contains(&self.tcp_keepalive_retries) {
                return Err(ConfigError::InvalidKeepalive(format!(
                    "--tcp-keepalive-retries must be between 1 and {MAX_TCP_KEEPALIVE_RETRIES} (got {})",
                    self.tcp_keepalive_retries
                )));
            }
        }
        Ok(())
    }

    /// The configured TCP keepalive parameters, or `None` when keepalive is disabled.
    pub fn tcp_keepalive_settings(&self) -> Option<KeepaliveSettings> {
        (!self.tcp_keepalive.is_zero()).then_some(KeepaliveSettings {
            idle: self.tcp_keepalive,
            interval: self.tcp_keepalive_interval,
            retries: self.tcp_keepalive_retries,
        })
    }

    /// The configured destination filter, or `None` when unfiltered.
    pub fn host_filter(&self) -> Option<HostFilter> {
        HostFilter::from_lists(self.allow.clone(), self.block.clone())
    }

    /// Engine routing input: the fixed `direct` default with no rules and no profiles,
    /// plus upstream TCP keepalive and the MITM flag.
    ///
    /// The destination filter is deliberately *not* set here: the router's pre-routing check
    /// cannot resolve DNS, so it would reject a hostname under an IP-only allowlist. As in
    /// v1.2.0, the filter is enforced at dial time instead (domain and resolved IPs), via
    /// [`crate::routing_state`] passing it to the direct dialer.
    pub fn routing_input(&self) -> RoutingInput {
        RoutingInput {
            filter: None,
            mitm_enabled: self.mitm_enabled(),
            listeners: self.server_options().listeners(),
            keepalive: self.tcp_keepalive_settings(),
            ..RoutingInput::default()
        }
    }

    /// Parse `mitm_insecure_hosts` into filter rules; the first invalid rule is an error.
    pub fn mitm_insecure_rules(&self) -> Result<Vec<FilterRule>, ConfigError> {
        self.mitm_insecure_hosts
            .iter()
            .map(|rule| {
                rule.parse::<FilterRule>()
                    .map_err(|message| ConfigError::InvalidMitmInsecureHost {
                        rule: rule.clone(),
                        message,
                    })
            })
            .collect()
    }

    /// Parse `mitm_bypass_hosts` into filter rules; the first invalid rule is an error.
    pub fn mitm_bypass_rules(&self) -> Result<Vec<FilterRule>, ConfigError> {
        self.mitm_bypass_hosts
            .iter()
            .map(|rule| {
                rule.parse::<FilterRule>()
                    .map_err(|message| ConfigError::InvalidMitmBypassHost {
                        rule: rule.clone(),
                        message,
                    })
            })
            .collect()
    }

    /// Upstream TLS verification mode: blind trust wins over the host list, else strict.
    ///
    /// Invalid host rules are rejected by [`Config::validate`]; any left on an unvalidated
    /// config are skipped here, so they never widen the insecure set.
    pub fn upstream_tls(&self) -> UpstreamTlsVerification {
        if self.mitm_insecure {
            UpstreamTlsVerification::InsecureAll
        } else if !self.mitm_insecure_hosts.is_empty() {
            UpstreamTlsVerification::AllowInsecureHosts(
                self.mitm_insecure_hosts
                    .iter()
                    .filter_map(|rule| rule.parse::<FilterRule>().ok())
                    .collect(),
            )
        } else {
            UpstreamTlsVerification::Strict
        }
    }

    /// Load the proxy error-page template: the `--error-page` file, or the built-in page.
    ///
    /// The error names the path, for a `FATAL` startup or `validate` message.
    pub fn error_page_template(&self) -> Result<ErrorPageTemplate, String> {
        match &self.error_page {
            Some(path) => ErrorPageTemplate::from_file(path)
                .map_err(|e| format!("cannot read --error-page {}: {e}", path.display())),
            None => Ok(ErrorPageTemplate::builtin()),
        }
    }

    /// Engine MITM settings (CA source, passphrase sources, leaf TTL, cache cap, upstream TLS,
    /// bypass rules).
    ///
    /// Invalid bypass rules are rejected by [`Config::validate`]; any left on an unvalidated
    /// config are skipped here.
    pub fn mitm_options(&self) -> MitmOptions {
        MitmOptions {
            ca: self.ca_source(),
            passphrase: self.mitm_ca_passphrase.clone(),
            passphrase_file: self.mitm_ca_passphrase_file.clone(),
            cert_ttl: self.mitm_cert_ttl,
            cert_cache_max: self.mitm_cert_cache_max_entries,
            upstream_tls: self.upstream_tls(),
            bypass_hosts: self
                .mitm_bypass_hosts
                .iter()
                .filter_map(|rule| rule.parse::<FilterRule>().ok())
                .collect(),
        }
    }

    /// Startup warnings about relaxed upstream TLS verification (empty unless MITM is on).
    pub fn mitm_tls_warnings(&self) -> Vec<String> {
        if !self.mitm_enabled() {
            return Vec::new();
        }
        if self.mitm_insecure {
            return vec![
                "WARN: MITM blind-trust mode (--mitm-insecure) is enabled: upstream HTTPS \
                 certificates will NOT be verified for any destination"
                    .to_string(),
            ];
        }
        if !self.mitm_insecure_hosts.is_empty() {
            return vec![format!(
                "WARN: MITM upstream TLS certificate verification is skipped for {} \
                 --mitm-insecure-host rule(s): {}",
                self.mitm_insecure_hosts.len(),
                self.mitm_insecure_hosts.join(",")
            )];
        }
        Vec::new()
    }

    /// Engine listener settings (bind, ports, connection cap, client TCP keepalive).
    pub fn server_options(&self) -> ServerOptions {
        ServerOptions {
            bind: self.bind,
            http_port: self.http_port,
            socks_port: self.socks_port,
            max_connections: self.max_connections,
            keepalive: self.tcp_keepalive_settings(),
        }
    }

    /// The configured MITM CA source, or `None` when MITM mode is OFF.
    pub fn ca_source(&self) -> Option<CaSource> {
        match (&self.mitm_ca_p12, &self.mitm_ca_cert, &self.mitm_ca_key) {
            (Some(path), None, None) => Some(CaSource::Pkcs12 { path: path.clone() }),
            (None, Some(cert), Some(key)) => Some(CaSource::Pem {
                cert: cert.clone(),
                key: key.clone(),
            }),
            _ => None,
        }
    }

    /// Whether HTTPS MITM inspection is enabled (a complete CA source is configured).
    pub fn mitm_enabled(&self) -> bool {
        self.ca_source().is_some()
    }

    /// Whether both listeners share one port (single-port first-byte multiplexing).
    pub fn is_multiplexed(&self) -> bool {
        self.http_port > 0 && self.http_port == self.socks_port
    }

    /// Whether the bind address is reachable beyond the local host.
    pub fn is_non_loopback_bind(&self) -> bool {
        !self.bind.is_loopback()
    }

    /// Resolve the CA passphrase from the inline value or the passphrase file
    /// (trailing `\n` / `\r\n` trimmed).
    pub fn resolve_passphrase(&self) -> Result<Option<String>, ConfigError> {
        if let Some(p) = &self.mitm_ca_passphrase {
            return Ok(Some(p.clone()));
        }
        match &self.mitm_ca_passphrase_file {
            Some(path) => {
                let text =
                    fs::read_to_string(path).map_err(|source| ConfigError::ReadPassphraseFile {
                        path: path.clone(),
                        source,
                    })?;
                let trimmed = text
                    .strip_suffix("\r\n")
                    .or_else(|| text.strip_suffix('\n'))
                    .unwrap_or(&text);
                Ok(Some(trimmed.to_string()))
            }
            None => Ok(None),
        }
    }

    /// Startup `INFO` notices (security and configuration hints), in display order.
    pub fn notices(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.is_non_loopback_bind() {
            out.push(format!(
                "SECURITY NOTICE: binding to non-loopback address {}; rusty-conduit has NO \
                 authentication, so anyone who can reach this address can use the proxy",
                self.bind
            ));
        }
        if self.passphrase_from_cli {
            out.push(
                "NOTICE: --mitm-ca-passphrase was passed on the command line, where it may be \
                 visible to other users and shell history; prefer RUSTY_CONDUIT_MITM_CA_PASSPHRASE \
                 or --mitm-ca-passphrase-file"
                    .to_string(),
            );
        }
        out.extend(self.file_notices.iter().cloned());
        out
    }

    /// One-line configuration summary for the startup banner (never includes secrets).
    pub fn summary(&self) -> String {
        fn port(p: u16) -> String {
            if p == 0 {
                "disabled".to_string()
            } else {
                p.to_string()
            }
        }
        fn path(p: &Option<PathBuf>) -> String {
            p.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "none".to_string())
        }
        fn cap(n: usize) -> String {
            if n == 0 {
                "unlimited".to_string()
            } else {
                n.to_string()
            }
        }
        let mitm = match self.ca_source() {
            Some(CaSource::Pem { .. }) => "on(pem)",
            Some(CaSource::Pkcs12 { .. }) => "on(pkcs12)",
            None => "off",
        };
        let keepalive = match self.tcp_keepalive_settings() {
            Some(ka) => format!(
                "{}/{}x{}",
                humantime::format_duration(ka.idle),
                humantime::format_duration(ka.interval),
                ka.retries
            ),
            None => "off".to_string(),
        };
        let upstream_tls = match self.upstream_tls() {
            UpstreamTlsVerification::Strict => "strict".to_string(),
            UpstreamTlsVerification::AllowInsecureHosts(rules) => {
                format!("insecure_hosts({})", rules.len())
            }
            UpstreamTlsVerification::InsecureAll => "insecure_all".to_string(),
        };
        let filter = self
            .host_filter()
            .map(|f| format!(" filter={}({})", f.mode_str(), f.len()))
            .unwrap_or_default();
        format!(
            "bind={} http_port={} socks_port={} multiplexed={} max_connections={} tcp_keepalive={keepalive} mitm={} \
             mitm_cert_ttl={} mitm_cert_cache_max_entries={} mitm_upstream_tls={upstream_tls} mitm_bypass_hosts={} socks_block_web={} error_page={} log_level={} log_format={} http_access_log={} socks_log={} \
             config={}{filter}",
            self.bind,
            port(self.http_port),
            port(self.socks_port),
            self.is_multiplexed(),
            cap(self.max_connections),
            mitm,
            humantime::format_duration(self.mitm_cert_ttl),
            cap(self.mitm_cert_cache_max_entries),
            self.mitm_bypass_hosts.len(),
            self.socks_block_web,
            self.error_page
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "builtin".to_string()),
            self.log_level,
            self.log_format,
            path(&self.http_access_log),
            path(&self.socks_log),
            path(&self.config_path),
        )
    }
}

fn file_duration(key: &'static str, raw: Option<&str>) -> Result<Option<Duration>, ConfigError> {
    raw.map(|s| {
        humantime::parse_duration(s).map_err(|e| ConfigError::InvalidFileValue {
            key,
            message: format!("{s:?}: {e}"),
        })
    })
    .transpose()
}

fn file_filter_rules(
    key: &'static str,
    raw: Option<Vec<String>>,
) -> Result<Vec<FilterRule>, ConfigError> {
    let Some(entries) = raw else {
        return Ok(Vec::new());
    };
    entries
        .into_iter()
        .map(|s| {
            s.parse::<FilterRule>()
                .map_err(|message| ConfigError::InvalidFileValue { key, message })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, MutexGuard};

    /// Serializes tests that read or mutate process environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const ENV_VARS: &[&str] = &[
        "RUSTY_CONDUIT_CONFIG",
        "RUSTY_CONDUIT_BIND",
        "RUSTY_CONDUIT_HTTP_PORT",
        "RUSTY_CONDUIT_SOCKS_PORT",
        "RUSTY_CONDUIT_MAX_CONNECTIONS",
        "RUSTY_CONDUIT_TCP_KEEPALIVE",
        "RUSTY_CONDUIT_TCP_KEEPALIVE_INTERVAL",
        "RUSTY_CONDUIT_TCP_KEEPALIVE_RETRIES",
        "RUSTY_CONDUIT_ALLOW",
        "RUSTY_CONDUIT_BLOCK",
        "RUSTY_CONDUIT_MITM_CERT_CACHE_MAX_ENTRIES",
        "RUSTY_CONDUIT_MITM_CA_CERT",
        "RUSTY_CONDUIT_MITM_CA_KEY",
        "RUSTY_CONDUIT_MITM_CA_P12",
        "RUSTY_CONDUIT_MITM_CA_PASSPHRASE",
        "RUSTY_CONDUIT_MITM_CA_PASSPHRASE_FILE",
        "RUSTY_CONDUIT_MITM_CERT_TTL",
        "RUSTY_CONDUIT_MITM_INSECURE_HOST",
        "RUSTY_CONDUIT_MITM_BYPASS_HOST",
        "RUSTY_CONDUIT_MITM_INSECURE",
        "RUSTY_CONDUIT_SOCKS_BLOCK_WEB",
        "RUSTY_CONDUIT_ERROR_PAGE",
        "RUSTY_CONDUIT_LOG_LEVEL",
        "RUSTY_CONDUIT_LOG_FORMAT",
        "RUSTY_CONDUIT_HTTP_ACCESS_LOG",
        "RUSTY_CONDUIT_SOCKS_LOG",
    ];

    /// Holds the env lock and restores a clean `RUSTY_CONDUIT_*` environment on drop.
    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn new(vars: &[(&str, &str)]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            for v in ENV_VARS {
                std::env::remove_var(v);
            }
            for (k, v) in vars {
                std::env::set_var(k, v);
            }
            Self { _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for v in ENV_VARS {
                std::env::remove_var(v);
            }
        }
    }

    fn temp_file(contents: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rusty-conduit-config-test-{}-{}.toml",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::write(&path, contents).unwrap();
        path
    }

    fn cli(args: &[&str]) -> CliInput {
        let mut full = vec!["rusty-conduit"];
        full.extend_from_slice(args);
        CliInput::try_parse_from(full).expect("cli parse")
    }

    fn load(args: &[&str]) -> Result<Config, ConfigError> {
        let cfg = Config::from_cli(cli(args))?;
        cfg.validate()?;
        Ok(cfg)
    }

    #[test]
    fn defaults_when_nothing_is_set() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.bind, "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(cfg.http_port, 3128);
        assert_eq!(cfg.socks_port, 1080);
        assert_eq!(cfg.mitm_cert_ttl, Duration::from_secs(3600));
        assert_eq!(cfg.log_level, LogLevel::Info);
        assert_eq!(cfg.log_format, LogFormat::Text);
        assert!(!cfg.socks_block_web);
        assert!(!cfg.mitm_enabled());
        assert!(!cfg.is_multiplexed());
        assert!(cfg.notices().is_empty());
    }

    const FULL_TOML: &str = r#"
bind = "10.0.0.1"
http_port = 1111
socks_port = 2222

socks_block_web = true

[mitm]
ca_p12 = "/file/ca.p12"
ca_passphrase_file = "/file/pass"
cert_ttl = "30m"

[logging]
level = "debug"
format = "json"
http_access_log = "/file/http.log"
socks_log = "/file/socks.log"
"#;

    #[test]
    fn toml_overrides_defaults() {
        let _env = EnvGuard::new(&[]);
        let path = temp_file(FULL_TOML);
        let cfg = load(&["--config", path.to_str().unwrap()]).unwrap();
        assert_eq!(cfg.bind, "10.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(cfg.http_port, 1111);
        assert_eq!(cfg.socks_port, 2222);
        assert_eq!(cfg.mitm_ca_p12, Some(PathBuf::from("/file/ca.p12")));
        assert_eq!(
            cfg.mitm_ca_passphrase_file,
            Some(PathBuf::from("/file/pass"))
        );
        assert_eq!(cfg.mitm_cert_ttl, Duration::from_secs(1800));
        assert!(cfg.socks_block_web);
        assert_eq!(cfg.log_level, LogLevel::Debug);
        assert_eq!(cfg.log_format, LogFormat::Json);
        assert_eq!(cfg.http_access_log, Some(PathBuf::from("/file/http.log")));
        assert_eq!(cfg.socks_log, Some(PathBuf::from("/file/socks.log")));
        assert_eq!(
            cfg.ca_source(),
            Some(CaSource::Pkcs12 {
                path: PathBuf::from("/file/ca.p12")
            })
        );
        fs::remove_file(path).ok();
    }

    #[test]
    fn env_overrides_toml() {
        let path = temp_file(FULL_TOML);
        let _env = EnvGuard::new(&[
            ("RUSTY_CONDUIT_CONFIG", path.to_str().unwrap()),
            ("RUSTY_CONDUIT_BIND", "::1"),
            ("RUSTY_CONDUIT_HTTP_PORT", "3333"),
            ("RUSTY_CONDUIT_SOCKS_PORT", "4444"),
            ("RUSTY_CONDUIT_MITM_CERT_TTL", "2h"),
            ("RUSTY_CONDUIT_SOCKS_BLOCK_WEB", "false"),
            ("RUSTY_CONDUIT_LOG_LEVEL", "VERBOSE"),
            ("RUSTY_CONDUIT_LOG_FORMAT", "text"),
            ("RUSTY_CONDUIT_HTTP_ACCESS_LOG", "/env/http.log"),
            ("RUSTY_CONDUIT_SOCKS_LOG", "/env/socks.log"),
            ("RUSTY_CONDUIT_MITM_CA_P12", "/env/ca.p12"),
        ]);
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.config_path, Some(path.clone()));
        assert_eq!(cfg.bind, "::1".parse::<IpAddr>().unwrap());
        assert_eq!(cfg.http_port, 3333);
        assert_eq!(cfg.socks_port, 4444);
        assert_eq!(cfg.mitm_cert_ttl, Duration::from_secs(7200));
        assert!(!cfg.socks_block_web, "env false overrides toml true");
        assert_eq!(cfg.log_level, LogLevel::Verbose);
        assert_eq!(cfg.log_format, LogFormat::Text);
        assert_eq!(cfg.http_access_log, Some(PathBuf::from("/env/http.log")));
        assert_eq!(cfg.socks_log, Some(PathBuf::from("/env/socks.log")));
        assert_eq!(cfg.mitm_ca_p12, Some(PathBuf::from("/env/ca.p12")));
        // Not set in env: falls through to TOML.
        assert_eq!(
            cfg.mitm_ca_passphrase_file,
            Some(PathBuf::from("/file/pass"))
        );
        fs::remove_file(path).ok();
    }

    #[test]
    fn cli_overrides_env_and_toml() {
        let path = temp_file(FULL_TOML);
        let _env = EnvGuard::new(&[
            ("RUSTY_CONDUIT_CONFIG", path.to_str().unwrap()),
            ("RUSTY_CONDUIT_BIND", "::1"),
            ("RUSTY_CONDUIT_HTTP_PORT", "3333"),
            ("RUSTY_CONDUIT_LOG_LEVEL", "VERBOSE"),
            ("RUSTY_CONDUIT_LOG_FORMAT", "text"),
            ("RUSTY_CONDUIT_MITM_CERT_TTL", "2h"),
        ]);
        let cfg = load(&[
            "--bind",
            "127.0.0.2",
            "--http-port",
            "5555",
            "--socks-port",
            "6666",
            "--log-level",
            "error",
            "--log-format",
            "JSON",
            "--mitm-cert-ttl",
            "3600s",
            "--http-access-log",
            "/cli/http.log",
        ])
        .unwrap();
        assert_eq!(cfg.bind, "127.0.0.2".parse::<IpAddr>().unwrap());
        assert_eq!(cfg.http_port, 5555);
        assert_eq!(cfg.socks_port, 6666);
        assert_eq!(cfg.log_level, LogLevel::Error);
        assert_eq!(cfg.log_format, LogFormat::Json);
        assert_eq!(cfg.mitm_cert_ttl, Duration::from_secs(3600));
        assert_eq!(cfg.http_access_log, Some(PathBuf::from("/cli/http.log")));
        // Not on CLI or env: from TOML.
        assert_eq!(cfg.socks_log, Some(PathBuf::from("/file/socks.log")));
        fs::remove_file(path).ok();
    }

    #[test]
    fn cli_config_path_overrides_env_config_path() {
        let env_path = temp_file("http_port = 1\n");
        let cli_path = temp_file("http_port = 2\n");
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_CONFIG", env_path.to_str().unwrap())]);
        let cfg = load(&["--config", cli_path.to_str().unwrap()]).unwrap();
        assert_eq!(cfg.http_port, 2);
        fs::remove_file(env_path).ok();
        fs::remove_file(cli_path).ok();
    }

    #[test]
    fn socks_block_web_flag_and_env() {
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_SOCKS_BLOCK_WEB", "true")]);
        let c = cli(&[]);
        assert!(c.socks_block_web_explicit);
        assert!(Config::merge(c, None).unwrap().socks_block_web);
        drop(_env);

        // The retired env var is ignored.
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_SOCKS_BLOCK_WEB_IN_MITM", "true")]);
        let c = cli(&[]);
        assert!(!c.socks_block_web_explicit);
        assert!(!Config::merge(c, None).unwrap().socks_block_web);
        drop(_env);

        let _env = EnvGuard::new(&[]);
        let c = cli(&["--socks-block-web"]);
        assert!(c.socks_block_web_explicit);
        assert!(Config::merge(c, None).unwrap().socks_block_web);

        // The retired flag is rejected.
        assert!(CliInput::try_parse_from(["rusty-conduit", "--socks-block-web-in-mitm"]).is_err());

        let c = cli(&[]);
        assert!(!c.socks_block_web_explicit);
        let file = FileConfig {
            socks_block_web: Some(true),
            ..Default::default()
        };
        assert!(Config::merge(c, Some(file)).unwrap().socks_block_web);

        // The retired `[mitm] socks_block_web` key is an unknown field.
        let err = FileConfig::from_toml_str(
            "[mitm]
socks_block_web = true
",
            Path::new("x.toml"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("socks_block_web"), "{err}");
    }

    #[test]
    fn passphrase_provenance() {
        let _env = EnvGuard::new(&[]);
        let cfg = Config::merge(cli(&["--mitm-ca-passphrase", "s3cret"]), None).unwrap();
        assert!(cfg.passphrase_from_cli);
        assert_eq!(cfg.mitm_ca_passphrase.as_deref(), Some("s3cret"));
        assert!(cfg
            .notices()
            .iter()
            .any(|n| n.contains("--mitm-ca-passphrase")));
        assert!(!format!("{cfg:?}").contains("s3cret"), "Debug must redact");
        drop(_env);

        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_MITM_CA_PASSPHRASE", "s3cret")]);
        let cfg = Config::merge(cli(&[]), None).unwrap();
        assert!(!cfg.passphrase_from_cli);
        assert_eq!(cfg.mitm_ca_passphrase.as_deref(), Some("s3cret"));
        assert!(cfg.notices().is_empty());
    }

    #[test]
    fn passphrase_file_trims_trailing_newline() {
        let path = temp_file("hunter2\n");
        let cfg = Config {
            mitm_ca_passphrase_file: Some(path.clone()),
            ..Config::default()
        };
        assert_eq!(
            cfg.resolve_passphrase().unwrap().as_deref(),
            Some("hunter2")
        );
        fs::write(&path, "hunter2\r\n").unwrap();
        assert_eq!(
            cfg.resolve_passphrase().unwrap().as_deref(),
            Some("hunter2")
        );
        fs::write(&path, "with space ").unwrap();
        assert_eq!(
            cfg.resolve_passphrase().unwrap().as_deref(),
            Some("with space ")
        );
        fs::remove_file(path).ok();
    }

    // ---- FATAL validation rules ----

    #[test]
    fn fatal_both_ports_disabled() {
        let _env = EnvGuard::new(&[]);
        let err = load(&["--http-port", "0", "--socks-port", "0"]).unwrap_err();
        assert!(matches!(err, ConfigError::NoListeners));
        // Disabling only one is fine.
        load(&["--http-port", "0"]).unwrap();
        load(&["--socks-port", "0"]).unwrap();
    }

    #[test]
    fn fatal_p12_combined_with_pem() {
        let _env = EnvGuard::new(&[]);
        let err = load(&["--mitm-ca-p12", "a.p12", "--mitm-ca-cert", "c.pem"]).unwrap_err();
        assert!(matches!(err, ConfigError::AmbiguousCaSource));
        let err = load(&["--mitm-ca-p12", "a.p12", "--mitm-ca-key", "k.pem"]).unwrap_err();
        assert!(matches!(err, ConfigError::AmbiguousCaSource));
        let err = load(&[
            "--mitm-ca-p12",
            "a.p12",
            "--mitm-ca-cert",
            "c.pem",
            "--mitm-ca-key",
            "k.pem",
        ])
        .unwrap_err();
        assert!(matches!(err, ConfigError::AmbiguousCaSource));
    }

    #[test]
    fn fatal_ambiguous_ca_across_sources() {
        let path = temp_file("[mitm]\nca_cert = \"c.pem\"\nca_key = \"k.pem\"\n");
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_MITM_CA_P12", "a.p12")]);
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(matches!(err, ConfigError::AmbiguousCaSource));
        fs::remove_file(path).ok();
    }

    #[test]
    fn fatal_incomplete_pem_pair() {
        let _env = EnvGuard::new(&[]);
        let err = load(&["--mitm-ca-cert", "c.pem"]).unwrap_err();
        assert!(matches!(err, ConfigError::IncompletePemCa));
        let err = load(&["--mitm-ca-key", "k.pem"]).unwrap_err();
        assert!(matches!(err, ConfigError::IncompletePemCa));
        let cfg = load(&["--mitm-ca-cert", "c.pem", "--mitm-ca-key", "k.pem"]).unwrap();
        assert!(cfg.mitm_enabled());
        assert_eq!(
            cfg.ca_source(),
            Some(CaSource::Pem {
                cert: "c.pem".into(),
                key: "k.pem".into()
            })
        );
    }

    #[test]
    fn fatal_both_passphrase_sources() {
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_MITM_CA_PASSPHRASE_FILE", "/pass")]);
        let err = load(&["--mitm-ca-passphrase", "x"]).unwrap_err();
        assert!(matches!(err, ConfigError::ConflictingPassphrase));
    }

    #[test]
    fn fatal_cert_ttl_below_10s() {
        let _env = EnvGuard::new(&[]);
        let err = load(&["--mitm-cert-ttl", "9s"]).unwrap_err();
        assert!(matches!(err, ConfigError::CertTtlTooShort(d) if d == Duration::from_secs(9)));
        let err = load(&["--mitm-cert-ttl", "9999ms"]).unwrap_err();
        assert!(matches!(err, ConfigError::CertTtlTooShort(_)));
        let cfg = load(&["--mitm-cert-ttl", "10s"]).unwrap();
        assert_eq!(cfg.mitm_cert_ttl, Duration::from_secs(10));
    }

    #[test]
    fn fatal_cert_ttl_below_10s_from_toml() {
        let path = temp_file("[mitm]\ncert_ttl = \"5s\"\n");
        let _env = EnvGuard::new(&[]);
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(matches!(err, ConfigError::CertTtlTooShort(_)));
        fs::remove_file(path).ok();
    }

    #[test]
    fn fatal_bad_config_file() {
        let _env = EnvGuard::new(&[]);
        let err = load(&["--config", "/definitely/not/here.toml"]).unwrap_err();
        assert!(matches!(err, ConfigError::ReadFile { .. }));

        let path = temp_file("unknown_key = 1\n");
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(matches!(err, ConfigError::ParseFile { .. }));
        fs::remove_file(path).ok();

        let path = temp_file("[logging]\nlevel = \"loud\"\n");
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidFileValue {
                key: "logging.level",
                ..
            }
        ));
        fs::remove_file(path).ok();

        let path = temp_file("[mitm]\ncert_ttl = \"forever\"\n");
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidFileValue {
                key: "mitm.cert_ttl",
                ..
            }
        ));
        fs::remove_file(path).ok();
    }

    #[test]
    fn invalid_cli_values_are_rejected() {
        let _env = EnvGuard::new(&[]);
        for args in [
            &["rusty-conduit", "--log-level", "loud"][..],
            &["rusty-conduit", "--log-format", "xml"][..],
            &["rusty-conduit", "--bind", "not-an-ip"][..],
            &["rusty-conduit", "--http-port", "70000"][..],
            &["rusty-conduit", "--mitm-cert-ttl", "soon"][..],
        ] {
            assert!(CliInput::try_parse_from(args).is_err(), "{args:?}");
        }
    }

    // ---- INFO notices ----

    #[test]
    fn notice_non_loopback_bind() {
        let _env = EnvGuard::new(&[]);
        for bind in ["0.0.0.0", "192.168.1.10", "::"] {
            let cfg = load(&["--bind", bind]).unwrap();
            assert!(cfg.is_non_loopback_bind(), "{bind}");
            assert!(cfg.notices()[0].contains("NO authentication"), "{bind}");
        }
        for bind in ["127.0.0.1", "::1"] {
            let cfg = load(&["--bind", bind]).unwrap();
            assert!(!cfg.is_non_loopback_bind(), "{bind}");
            assert!(cfg.notices().is_empty(), "{bind}");
        }
    }

    #[test]
    fn socks_block_web_works_without_mitm_no_inactive_notice() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&["--socks-block-web"]).unwrap();
        assert!(cfg.socks_block_web);
        assert!(cfg.notices().is_empty());
    }

    #[test]
    fn multiplexed_when_ports_equal() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&["--http-port", "8080", "--socks-port", "8080"]).unwrap();
        assert!(cfg.is_multiplexed());
        assert!(cfg.summary().contains("multiplexed=true"));
    }

    // ---- Resource caps ----

    const CAPS_TOML: &str = "max_connections = 11\n[mitm]\ncert_cache_max_entries = 22\n";

    #[test]
    fn caps_defaults() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.max_connections, 4096);
        assert_eq!(cfg.mitm_cert_cache_max_entries, 10_240);
        let s = cfg.summary();
        assert!(s.contains("max_connections=4096"), "{s}");
        assert!(s.contains("mitm_cert_cache_max_entries=10240"), "{s}");
    }

    #[test]
    fn caps_from_toml() {
        let path = temp_file(CAPS_TOML);
        let _env = EnvGuard::new(&[]);
        let cfg = load(&["--config", path.to_str().unwrap()]).unwrap();
        assert_eq!(cfg.max_connections, 11);
        assert_eq!(cfg.mitm_cert_cache_max_entries, 22);
        fs::remove_file(path).ok();
    }

    #[test]
    fn caps_env_overrides_toml() {
        let path = temp_file(CAPS_TOML);
        let _env = EnvGuard::new(&[
            ("RUSTY_CONDUIT_MAX_CONNECTIONS", "33"),
            ("RUSTY_CONDUIT_MITM_CERT_CACHE_MAX_ENTRIES", "44"),
        ]);
        let cfg = load(&["--config", path.to_str().unwrap()]).unwrap();
        assert_eq!(cfg.max_connections, 33);
        assert_eq!(cfg.mitm_cert_cache_max_entries, 44);
        fs::remove_file(path).ok();
    }

    #[test]
    fn caps_cli_overrides_env_and_toml() {
        let path = temp_file(CAPS_TOML);
        let _env = EnvGuard::new(&[
            ("RUSTY_CONDUIT_MAX_CONNECTIONS", "33"),
            ("RUSTY_CONDUIT_MITM_CERT_CACHE_MAX_ENTRIES", "44"),
        ]);
        let cfg = load(&[
            "--config",
            path.to_str().unwrap(),
            "--max-connections",
            "55",
            "--mitm-cert-cache-max-entries",
            "66",
        ])
        .unwrap();
        assert_eq!(cfg.max_connections, 55);
        assert_eq!(cfg.mitm_cert_cache_max_entries, 66);
        fs::remove_file(path).ok();
    }

    #[test]
    fn caps_zero_means_unlimited_at_every_layer() {
        let path = temp_file("max_connections = 0\n[mitm]\ncert_cache_max_entries = 0\n");
        let _env = EnvGuard::new(&[]);
        let cfg = load(&["--config", path.to_str().unwrap()]).unwrap();
        assert_eq!(cfg.max_connections, 0);
        assert_eq!(cfg.mitm_cert_cache_max_entries, 0);
        let s = cfg.summary();
        assert!(s.contains("max_connections=unlimited"), "{s}");
        assert!(s.contains("mitm_cert_cache_max_entries=unlimited"), "{s}");
        fs::remove_file(path).ok();
        drop(_env);

        // An explicit CLI 0 beats a non-zero env value rather than falling through.
        let _env = EnvGuard::new(&[
            ("RUSTY_CONDUIT_MAX_CONNECTIONS", "33"),
            ("RUSTY_CONDUIT_MITM_CERT_CACHE_MAX_ENTRIES", "44"),
        ]);
        let cfg = load(&[
            "--max-connections",
            "0",
            "--mitm-cert-cache-max-entries",
            "0",
        ])
        .unwrap();
        assert_eq!(cfg.max_connections, 0);
        assert_eq!(cfg.mitm_cert_cache_max_entries, 0);
    }

    #[test]
    fn caps_reject_invalid_values() {
        let _env = EnvGuard::new(&[]);
        for args in [
            &["rusty-conduit", "--max-connections", "-1"][..],
            &["rusty-conduit", "--max-connections", "many"][..],
            &["rusty-conduit", "--mitm-cert-cache-max-entries", "-1"][..],
            &["rusty-conduit", "--mitm-cert-cache-max-entries", "many"][..],
        ] {
            assert!(CliInput::try_parse_from(args).is_err(), "{args:?}");
        }
        let path = temp_file("max_connections = -1\n");
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(matches!(err, ConfigError::ParseFile { .. }), "{err}");
        fs::remove_file(path).ok();
    }

    #[test]
    fn summary_never_contains_passphrase() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&["--mitm-ca-p12", "a.p12", "--mitm-ca-passphrase", "s3cret"]).unwrap();
        let s = cfg.summary();
        assert!(s.contains("mitm=on(pkcs12)"));
        assert!(!s.contains("s3cret"));
    }

    #[test]
    fn filter_cli_env_toml_and_conflict_validation() {
        let _env = EnvGuard::new(&[]);
        // CLI repeatable + comma-separated
        let cfg = load(&[
            "--allow",
            "*.example.com,10.0.0.0/8",
            "--allow",
            "127.0.0.1",
        ])
        .unwrap();
        assert_eq!(cfg.allow.len(), 3);
        assert!(cfg.block.is_empty());
        assert!(matches!(cfg.host_filter(), Some(HostFilter::Allow(_))));
        let s = cfg.summary();
        assert!(s.contains("filter=allow(3)"), "{s}");

        // Combined allow + block: allowlist serves as exceptions to blocklist
        let cfg = load(&["--allow", "test.domain.net", "--block", "*.domain.net"]).unwrap();
        assert!(matches!(
            cfg.host_filter(),
            Some(HostFilter::BlockExcept { .. })
        ));
        assert!(cfg.summary().contains("filter=block-except(2)"));

        // TOML parsing
        let toml_path = temp_file(
            r#"
allow = ["example.com", "192.168.0.0/16"]
"#,
        );
        let cfg = load(&["--config", toml_path.to_str().unwrap()]).unwrap();
        assert_eq!(cfg.allow.len(), 2);
        assert!(cfg.block.is_empty());

        // Cross-source combination (TOML allow + CLI block) produces BlockExcept
        let cfg = load(&[
            "--config",
            toml_path.to_str().unwrap(),
            "--block",
            "10.0.0.0/8",
        ])
        .unwrap();
        assert!(matches!(
            cfg.host_filter(),
            Some(HostFilter::BlockExcept { .. })
        ));

        // Env var override
        drop(_env);
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_BLOCK", "127.0.0.0/8,::1,*.internal")]);
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.block.len(), 3);
        assert!(matches!(cfg.host_filter(), Some(HostFilter::Block(_))));
        assert!(cfg.summary().contains("filter=block(3)"));

        // Invalid TOML rule fails with InvalidFileValue
        let bad_toml = temp_file("block = [\"foo.*.example.com\"]\n");
        let err = load(&["--config", bad_toml.to_str().unwrap()]).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidFileValue { key: "block", .. }),
            "{err:?}"
        );

        let _ = fs::remove_file(&toml_path);
        let _ = fs::remove_file(&bad_toml);
    }

    // ---- TCP keepalive ----

    #[test]
    fn keepalive_defaults() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&[]).unwrap();
        assert_eq!(
            cfg.tcp_keepalive_settings(),
            Some(KeepaliveSettings::DEFAULT)
        );
        assert_eq!(KeepaliveSettings::DEFAULT.idle, Duration::from_secs(60));
        assert_eq!(KeepaliveSettings::DEFAULT.interval, Duration::from_secs(10));
        assert_eq!(KeepaliveSettings::DEFAULT.retries, 5);
        let s = cfg.summary();
        assert!(s.contains("tcp_keepalive=1m/10sx5"), "{s}");
    }

    #[test]
    fn keepalive_precedence_cli_over_env_over_toml() {
        let path = temp_file(
            "tcp_keepalive = \"2m\"\ntcp_keepalive_interval = \"20s\"\ntcp_keepalive_retries = 3\n",
        );
        let p = path.to_str().unwrap();

        let _env = EnvGuard::new(&[]);
        let cfg = load(&["--config", p]).unwrap();
        assert_eq!(
            cfg.tcp_keepalive_settings(),
            Some(KeepaliveSettings {
                idle: Duration::from_secs(120),
                interval: Duration::from_secs(20),
                retries: 3,
            })
        );
        drop(_env);

        let _env = EnvGuard::new(&[
            ("RUSTY_CONDUIT_TCP_KEEPALIVE", "90s"),
            ("RUSTY_CONDUIT_TCP_KEEPALIVE_RETRIES", "7"),
        ]);
        let cfg = load(&["--config", p, "--tcp-keepalive-retries", "9"]).unwrap();
        assert_eq!(cfg.tcp_keepalive, Duration::from_secs(90));
        assert_eq!(cfg.tcp_keepalive_interval, Duration::from_secs(20));
        assert_eq!(cfg.tcp_keepalive_retries, 9);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn keepalive_zero_disables() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&["--tcp-keepalive", "0"]).unwrap();
        assert_eq!(cfg.tcp_keepalive_settings(), None);
        assert!(cfg.summary().contains("tcp_keepalive=off"));
        // Interval / retries are not validated while keepalive is off.
        let cfg = load(&["--tcp-keepalive", "0", "--tcp-keepalive-retries", "0"]).unwrap();
        assert_eq!(cfg.tcp_keepalive_settings(), None);

        let path = temp_file("tcp_keepalive = \"0s\"\n");
        let cfg = load(&["--config", path.to_str().unwrap()]).unwrap();
        assert_eq!(cfg.tcp_keepalive_settings(), None);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn fatal_invalid_keepalive() {
        let _env = EnvGuard::new(&[]);
        for args in [
            &["--tcp-keepalive", "500ms"][..],
            &["--tcp-keepalive", "10h"][..],
            &["--tcp-keepalive-interval", "0s"][..],
            &["--tcp-keepalive-retries", "0"][..],
            &["--tcp-keepalive-retries", "128"][..],
        ] {
            let err = load(args).unwrap_err();
            assert!(
                matches!(err, ConfigError::InvalidKeepalive(_)),
                "{args:?}: {err}"
            );
        }
        let path = temp_file("tcp_keepalive = \"soon\"\n");
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(matches!(
            err,
            ConfigError::InvalidFileValue {
                key: "tcp_keepalive",
                ..
            }
        ));
        let _ = fs::remove_file(&path);
    }

    fn rules(raw: &[&str]) -> Vec<FilterRule> {
        raw.iter().map(|r| r.parse().unwrap()).collect()
    }

    #[test]
    fn mitm_insecure_cli_flags() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&[]).unwrap();
        assert!(!cfg.mitm_insecure);
        assert!(cfg.mitm_insecure_hosts.is_empty());

        let cfg = load(&["--mitm-insecure"]).unwrap();
        assert!(cfg.mitm_insecure);

        let cfg = load(&[
            "--mitm-insecure-host",
            "dev.local",
            "--mitm-insecure-host",
            "10.0.0.0/8,*.lab.example",
        ])
        .unwrap();
        assert_eq!(
            cfg.mitm_insecure_hosts,
            vec!["dev.local", "10.0.0.0/8", "*.lab.example"]
        );
        assert!(!cfg.mitm_insecure);
    }

    #[test]
    fn mitm_insecure_env_vars() {
        let _env = EnvGuard::new(&[
            ("RUSTY_CONDUIT_MITM_INSECURE", "true"),
            ("RUSTY_CONDUIT_MITM_INSECURE_HOST", "a.example,192.0.2.7"),
        ]);
        let cfg = load(&[]).unwrap();
        assert!(cfg.mitm_insecure);
        assert_eq!(cfg.mitm_insecure_hosts, vec!["a.example", "192.0.2.7"]);
    }

    #[test]
    fn mitm_insecure_toml_and_precedence() {
        let _env = EnvGuard::new(&[]);
        let path = temp_file(
            "[mitm]\ninsecure = true\ninsecure_hosts = [\"file.example\", \"172.16.0.0/12\"]\n",
        );
        let p = path.to_str().unwrap();
        let cfg = load(&["--config", p]).unwrap();
        assert!(cfg.mitm_insecure);
        assert_eq!(
            cfg.mitm_insecure_hosts,
            vec!["file.example", "172.16.0.0/12"]
        );

        // CLI hosts replace the file list, as --allow / --block do.
        let cfg = load(&["--config", p, "--mitm-insecure-host", "cli.example"]).unwrap();
        assert_eq!(cfg.mitm_insecure_hosts, vec!["cli.example"]);
        let _ = fs::remove_file(&path);

        // Blind trust is CLI OR file.
        let path = temp_file("[mitm]\ninsecure = false\n");
        let p = path.to_str().unwrap();
        assert!(!load(&["--config", p]).unwrap().mitm_insecure);
        assert!(
            load(&["--config", p, "--mitm-insecure"])
                .unwrap()
                .mitm_insecure
        );
        let _ = fs::remove_file(&path);

        let file = FileConfig::from_toml_str("[mitm]\n", Path::new("x.toml")).unwrap();
        assert!(file.mitm.insecure_hosts.is_empty());
        assert_eq!(file.mitm.insecure, None);
    }

    #[test]
    fn fatal_invalid_mitm_insecure_host() {
        let _env = EnvGuard::new(&[]);
        let err = load(&["--mitm-insecure-host", "10.0.0.0/99"]).unwrap_err();
        assert!(
            matches!(&err, ConfigError::InvalidMitmInsecureHost { rule, .. } if rule == "10.0.0.0/99"),
            "{err}"
        );

        let path = temp_file("[mitm]\ninsecure_hosts = [\"ok.example\", \"bad host!\"]\n");
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(
            matches!(&err, ConfigError::InvalidMitmInsecureHost { rule, .. } if rule == "bad host!"),
            "{err}"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn mitm_bypass_cli_flags_and_env() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&[]).unwrap();
        assert!(cfg.mitm_bypass_hosts.is_empty());
        assert!(Config::default().mitm_bypass_hosts.is_empty());

        let cfg = load(&[
            "--mitm-bypass-host",
            "bank.example",
            "--mitm-bypass-host",
            "192.0.2.0/24,*.pinned.example",
        ])
        .unwrap();
        assert_eq!(
            cfg.mitm_bypass_hosts,
            vec!["bank.example", "192.0.2.0/24", "*.pinned.example"]
        );
        // Bypass rules are independent of the upstream TLS trust settings.
        assert!(cfg.mitm_insecure_hosts.is_empty());
        assert!(!cfg.mitm_insecure);
        drop(_env);

        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_MITM_BYPASS_HOST", "a.example,198.51.100.9")]);
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.mitm_bypass_hosts, vec!["a.example", "198.51.100.9"]);
        // The CLI flag wins over the environment variable.
        let cfg = load(&["--mitm-bypass-host", "cli.example"]).unwrap();
        assert_eq!(cfg.mitm_bypass_hosts, vec!["cli.example"]);
    }

    #[test]
    fn mitm_bypass_toml_and_precedence() {
        let _env = EnvGuard::new(&[]);
        let path = temp_file(
            "[mitm]\nbypass_hosts = [\"file.example\", \"*.corp.example\", \"10.1.0.0/16\"]\n",
        );
        let p = path.to_str().unwrap();
        let cfg = load(&["--config", p]).unwrap();
        assert_eq!(
            cfg.mitm_bypass_hosts,
            vec!["file.example", "*.corp.example", "10.1.0.0/16"]
        );

        // CLI hosts replace the file list, as --mitm-insecure-host does.
        let cfg = load(&["--config", p, "--mitm-bypass-host", "cli.example"]).unwrap();
        assert_eq!(cfg.mitm_bypass_hosts, vec!["cli.example"]);
        drop(_env);

        // So do env hosts.
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_MITM_BYPASS_HOST", "env.example")]);
        let cfg = load(&["--config", p]).unwrap();
        assert_eq!(cfg.mitm_bypass_hosts, vec!["env.example"]);
        let _ = fs::remove_file(&path);

        let file = FileConfig::from_toml_str("[mitm]\n", Path::new("x.toml")).unwrap();
        assert!(file.mitm.bypass_hosts.is_empty());
        let file = FileConfig::from_toml_str(
            "[mitm]\nbypass_hosts = [\"x.example\"]\n",
            Path::new("x.toml"),
        )
        .unwrap();
        assert_eq!(file.mitm.bypass_hosts, vec!["x.example"]);
    }

    #[test]
    fn deprecated_cert_cache_max_key_is_accepted_with_a_notice() {
        let _env = EnvGuard::new(&[]);
        let path = temp_file("[mitm]\ncert_cache_max = 77\n");
        let p = path.to_str().unwrap();
        let cfg = load(&["--config", p]).unwrap();
        assert_eq!(cfg.mitm_cert_cache_max_entries, 77);
        let notices = cfg.notices();
        assert!(
            notices
                .iter()
                .any(|n| n.contains("mitm.cert_cache_max is deprecated")),
            "{notices:?}"
        );
        let _ = fs::remove_file(&path);

        // The current key wins over the deprecated one; the notice is still shown.
        let path = temp_file("[mitm]\ncert_cache_max = 77\ncert_cache_max_entries = 88\n");
        let p = path.to_str().unwrap();
        let cfg = load(&["--config", p]).unwrap();
        assert_eq!(cfg.mitm_cert_cache_max_entries, 88);
        assert_eq!(cfg.file_notices.len(), 1);
        // And the command line wins over both.
        let cfg = load(&["--config", p, "--mitm-cert-cache-max-entries", "99"]).unwrap();
        assert_eq!(cfg.mitm_cert_cache_max_entries, 99);
        let _ = fs::remove_file(&path);

        let cfg = load(&[]).unwrap();
        assert!(cfg.file_notices.is_empty());
    }

    #[test]
    fn file_config_debug_hides_the_ca_passphrase() {
        let file = FileConfig::from_toml_str(
            "[mitm]\nca_passphrase = \"file-canary-secret\"\n",
            Path::new("x.toml"),
        )
        .unwrap();
        let rendered = format!("{file:?}");
        assert!(!rendered.contains("file-canary-secret"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn fatal_invalid_mitm_bypass_host() {
        let _env = EnvGuard::new(&[]);
        let err = load(&["--mitm-bypass-host", "10.0.0.0/99"]).unwrap_err();
        assert!(
            matches!(&err, ConfigError::InvalidMitmBypassHost { rule, .. } if rule == "10.0.0.0/99"),
            "{err}"
        );
        assert!(err.to_string().contains("--mitm-bypass-host"), "{err}");

        let path = temp_file("[mitm]\nbypass_hosts = [\"ok.example\", \"bad host!\"]\n");
        let err = load(&["--config", path.to_str().unwrap()]).unwrap_err();
        assert!(
            matches!(&err, ConfigError::InvalidMitmBypassHost { rule, .. } if rule == "bad host!"),
            "{err}"
        );
        let _ = fs::remove_file(&path);

        let cfg = Config {
            mitm_bypass_hosts: vec!["ok.example".into(), "bad host!".into()],
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
        assert!(cfg.mitm_bypass_rules().is_err());
    }

    #[test]
    fn mitm_options_bypass_hosts_and_summary() {
        let cfg = Config::default();
        assert!(cfg.mitm_options().bypass_hosts.is_empty());
        assert_eq!(cfg.mitm_bypass_rules().unwrap(), Vec::<FilterRule>::new());
        assert!(
            cfg.summary().contains("mitm_bypass_hosts=0"),
            "{}",
            cfg.summary()
        );

        let cfg = Config {
            mitm_bypass_hosts: vec![
                "bank.example".into(),
                "*.pinned.example".into(),
                "10.0.0.0/8".into(),
            ],
            ..Config::default()
        };
        let expected = rules(&["bank.example", "*.pinned.example", "10.0.0.0/8"]);
        assert_eq!(cfg.mitm_options().bypass_hosts, expected);
        assert_eq!(cfg.mitm_bypass_rules().unwrap(), expected);
        // Bypass rules do not relax upstream verification.
        assert_eq!(
            cfg.mitm_options().upstream_tls,
            UpstreamTlsVerification::Strict
        );
        let summary = cfg.summary();
        assert!(summary.contains("mitm_bypass_hosts=3"), "{summary}");
        assert!(
            format!("{cfg:?}").contains(
                "mitm_bypass_hosts: [\"bank.example\", \"*.pinned.example\", \"10.0.0.0/8\"]"
            ),
            "{cfg:?}"
        );

        // An unvalidated config never maps an invalid rule onto the engine.
        let cfg = Config {
            mitm_bypass_hosts: vec!["bad host!".into(), "ok.example".into()],
            ..Config::default()
        };
        assert_eq!(cfg.mitm_options().bypass_hosts, rules(&["ok.example"]));
    }

    #[test]
    fn app_version_reports_release_or_dev() {
        let version = app_version();
        match option_env!("CONDUIT_RELEASE_VERSION") {
            Some(release) if !release.is_empty() => assert_eq!(version, release),
            _ => assert_eq!(version, concat!(env!("CARGO_PKG_VERSION"), "-dev")),
        }
        assert!(version.starts_with(env!("CARGO_PKG_VERSION")), "{version}");
        assert_eq!(CliArgs::command().get_version(), Some(version));
        let err = CliArgs::try_parse_from(["rusty-conduit", "--version"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        assert_eq!(err.to_string().trim(), format!("rusty-conduit {version}"));
    }

    #[test]
    fn mitm_options_upstream_tls_modes() {
        let cfg = Config::default();
        assert_eq!(
            cfg.mitm_options().upstream_tls,
            UpstreamTlsVerification::Strict
        );
        assert!(cfg.summary().contains("mitm_upstream_tls=strict"));

        let cfg = Config {
            mitm_insecure_hosts: vec!["dev.local".into(), "10.0.0.0/8".into()],
            ..Config::default()
        };
        assert_eq!(
            cfg.mitm_options().upstream_tls,
            UpstreamTlsVerification::AllowInsecureHosts(rules(&["dev.local", "10.0.0.0/8"]))
        );
        assert!(cfg
            .summary()
            .contains("mitm_upstream_tls=insecure_hosts(2)"));

        let cfg = Config {
            mitm_insecure: true,
            mitm_insecure_hosts: vec!["dev.local".into()],
            ..Config::default()
        };
        assert_eq!(
            cfg.mitm_options().upstream_tls,
            UpstreamTlsVerification::InsecureAll
        );
        assert!(cfg.summary().contains("mitm_upstream_tls=insecure_all"));
    }

    #[test]
    fn mitm_tls_warnings_only_with_mitm_enabled() {
        let mitm = Config {
            mitm_ca_p12: Some(PathBuf::from("/ca.p12")),
            ..Config::default()
        };
        assert!(mitm.mitm_tls_warnings().is_empty());

        let off = Config {
            mitm_insecure: true,
            ..Config::default()
        };
        assert!(off.mitm_tls_warnings().is_empty());

        let all = Config {
            mitm_insecure: true,
            ..mitm.clone()
        };
        let w = all.mitm_tls_warnings();
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("blind-trust mode (--mitm-insecure)"), "{w:?}");

        let hosts = Config {
            mitm_insecure_hosts: vec!["a.example".into(), "10.0.0.0/8".into()],
            ..mitm
        };
        let w = hosts.mitm_tls_warnings();
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("skipped for 2"), "{w:?}");
    }

    #[test]
    fn error_page_cli_env_toml_and_precedence() {
        let _env = EnvGuard::new(&[]);
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.error_page, None);
        assert!(
            cfg.summary().contains(" error_page=builtin "),
            "{}",
            cfg.summary()
        );
        assert!(cfg.error_page_template().unwrap().is_builtin());

        let cfg = load(&["--error-page", "/cli/page.html"]).unwrap();
        assert_eq!(cfg.error_page, Some(PathBuf::from("/cli/page.html")));
        assert!(cfg.summary().contains(" error_page=/cli/page.html "));
        assert!(format!("{cfg:?}").contains("error_page: Some(\"/cli/page.html\")"));

        let path = temp_file("error_page = \"/file/page.html\"\n");
        let p = path.to_str().unwrap();
        let cfg = load(&["--config", p]).unwrap();
        assert_eq!(cfg.error_page, Some(PathBuf::from("/file/page.html")));

        // CLI overrides the file.
        let cfg = load(&["--config", p, "--error-page", "/cli/page.html"]).unwrap();
        assert_eq!(cfg.error_page, Some(PathBuf::from("/cli/page.html")));
        drop(_env);

        // Env overrides the file; CLI overrides env.
        let _env = EnvGuard::new(&[("RUSTY_CONDUIT_ERROR_PAGE", "/env/page.html")]);
        let cfg = load(&[]).unwrap();
        assert_eq!(cfg.error_page, Some(PathBuf::from("/env/page.html")));
        let cfg = load(&["--config", p]).unwrap();
        assert_eq!(cfg.error_page, Some(PathBuf::from("/env/page.html")));
        let cfg = load(&["--config", p, "--error-page", "/cli/page.html"]).unwrap();
        assert_eq!(cfg.error_page, Some(PathBuf::from("/cli/page.html")));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn error_page_template_loads_file_or_names_missing_path() {
        let _env = EnvGuard::new(&[]);
        let page = temp_file("<h1>{{status}} {{reason}}</h1>");
        let cfg = load(&["--error-page", page.to_str().unwrap()]).unwrap();
        let template = cfg.error_page_template().unwrap();
        assert!(!template.is_builtin());
        assert_eq!(template.as_str(), "<h1>{{status}} {{reason}}</h1>");
        let _ = fs::remove_file(&page);

        let missing = std::env::temp_dir().join("rusty-conduit-no-such-error-page.html");
        let cfg = load(&["--error-page", missing.to_str().unwrap()]).unwrap();
        let err = cfg.error_page_template().unwrap_err();
        assert!(
            err.starts_with(&format!("cannot read --error-page {}", missing.display())),
            "{err}"
        );
    }
}
