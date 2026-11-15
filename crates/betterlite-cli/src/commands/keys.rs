use std::convert::Infallible;
use std::io::Write;

use betterlite_redact::secret::Redactor;
use clap::builder::ValueParser;
use clap::{Args, Subcommand};
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::cli_context::CliContext;

use super::Client;
use super::tables::OutputFormat;

pub mod api;
mod client;
mod dates;
mod delete;
mod generate;
mod import;
mod list;
pub(in crate::commands) mod output;
mod wire;

#[derive(Debug, Subcommand)]
#[command(about = "Manage API keys for the proxy server")]
pub enum KeysCommand {
    List(ListKeys),
    Generate(GenerateKey),
    Delete(DeleteKeys),
    Import(ImportKeys),
}

#[derive(Debug, Args)]
pub struct ListKeys {
    #[arg(long, help = "Page number for pagination")]
    page: Option<i64>,
    #[arg(long, help = "Number of items per page")]
    size: Option<i64>,
    #[arg(long, help = "Filter keys by user ID")]
    user_id: Option<String>,
    #[arg(long, help = "Filter keys by team ID")]
    team_id: Option<String>,
    #[arg(long, help = "Filter keys by organization ID")]
    organization_id: Option<String>,
    #[arg(long, help = "Filter by specific key hash")]
    key_hash: Option<String>,
    #[arg(long, help = "Filter by key alias")]
    key_alias: Option<String>,
    #[arg(
        long = "return-full-object",
        default_value_t = true,
        help = "Return the full key object"
    )]
    return_full_object: bool,
    #[arg(long = "include-team-keys", help = "Include team keys in the response")]
    include_team_keys: bool,
    #[arg(
        long,
        value_enum,
        default_value_t = OutputFormat::Table,
        help = "Output format (table or json)"
    )]
    format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct GenerateKey {
    #[arg(long, help = "Comma-separated list of allowed models")]
    models: Option<String>,
    #[arg(long, help = "JSON string of model alias mappings")]
    aliases: Option<String>,
    #[arg(long, help = "Maximum spend limit for this key")]
    spend: Option<Decimal>,
    #[arg(long, help = "Duration for which the key is valid (e.g. '24h', '7d')")]
    duration: Option<String>,
    #[arg(long, help = "Alias/name for the key")]
    key_alias: Option<String>,
    #[arg(long, help = "Team ID to associate the key with")]
    team_id: Option<String>,
    #[arg(long, help = "User ID to associate the key with")]
    user_id: Option<String>,
    #[arg(long, help = "Budget ID to associate the key with")]
    budget_id: Option<String>,
    #[arg(long, help = "JSON string of additional configuration parameters")]
    config: Option<String>,
}

#[derive(Debug, Args)]
pub struct DeleteKeys {
    #[arg(long, help = "Comma-separated list of API keys to delete")]
    keys: Option<String>,
    #[arg(long, help = "Comma-separated list of key aliases to delete")]
    key_aliases: Option<String>,
}

#[derive(Debug, Args)]
pub struct ImportKeys {
    #[arg(
        long = "source-base-url",
        required = true,
        help = "Base URL of the source proxy server to import keys from"
    )]
    source_base_url: String,
    #[arg(
        long = "source-api-key",
        help = "API key for authentication to the source server",
        value_parser = ValueParser::new(|value: &str| -> Result<SecretString, Infallible> {
            Ok(SecretString::from(value))
        })
    )]
    source_api_key: Option<SecretString>,
    #[arg(
        long = "dry-run",
        help = "Show what would be imported without actually importing"
    )]
    dry_run: bool,
    #[arg(
        long = "created-since",
        help = "Only import keys created after this date/time (format: YYYY-MM-DD_HH:MM or YYYY-MM-DD)"
    )]
    created_since: Option<String>,
}

pub struct KeysPorts<'run> {
    pub context: &'run CliContext,
    pub client: &'run Client,
    pub redactor: &'run Redactor,
    pub stdout: &'run mut dyn Write,
    pub stderr: &'run mut dyn Write,
}

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("Invalid JSON: {source}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the --{option} option has the wrong type: {source}")]
    OptionType {
        option: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the --spend option is not a JSON number")]
    Spend {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum KeysError {
    #[error("the command was aborted")]
    Aborted,
    #[error(transparent)]
    Usage(#[from] UsageError),
    #[error("the response from {url} is not a JSON object")]
    NotAnObject { url: String },
    #[error("the response field {field} is not valid: {source}")]
    Field {
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the response for page {page} is not a JSON object")]
    PageNotAnObject { page: u64 },
    #[error("the field {field} of page {page} is not valid: {source}")]
    PageField {
        page: u64,
        field: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the created_at value '{value}' is not a date-time: {source}")]
    CreatedAt {
        value: String,
        #[source]
        source: chrono::ParseError,
    },
    #[error("the page number overflowed")]
    PageOverflow,
    #[error("cannot encode the JSON text")]
    Json(#[source] serde_json::Error),
    #[error(transparent)]
    Client(#[from] api::ApiError),
}

pub fn run_keys_command(command: KeysCommand, ports: KeysPorts<'_>) -> Result<(), KeysError> {
    match command {
        KeysCommand::List(options) => list::list_keys(&options, ports),
        KeysCommand::Generate(options) => generate::generate_key(&options, ports),
        KeysCommand::Delete(options) => delete::delete_keys(&options, ports),
        KeysCommand::Import(options) => import::import_keys(&options, ports),
    }
}

pub(super) fn split_commas(text: Option<&str>) -> Option<Vec<String>> {
    let value = text?;
    let parts = value
        .split(',')
        .map(|part| part.trim().to_owned())
        .collect();
    Some(parts)
}

mod tests;
