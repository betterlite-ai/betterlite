use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::Client;
use super::tables::OutputFormat;

mod add;
mod delete;
mod format;
mod get;
mod import;
mod info;
mod list;
mod params;
mod response;
mod update;
mod value;

#[derive(Debug, Subcommand)]
#[command(about = "Manage models on your LiteLLM proxy server")]
pub enum ModelsCommand {
    #[command(about = "List all available models")]
    List(ListModels),
    #[command(about = "Add a new model to the proxy")]
    Add(AddModel),
    #[command(about = "Delete a model from the proxy")]
    Delete(DeleteModel),
    #[command(about = "Get information about a specific model")]
    Get(GetModel),
    #[command(about = "Get detailed information about all models")]
    Info(GetModelsInfo),
    #[command(about = "Update an existing model's configuration")]
    Update(UpdateModel),
    #[command(about = "Import models from a YAML file and add them to the proxy")]
    Import(ImportModels),
}

#[derive(Debug, Args)]
pub struct ListModels {
    #[arg(
        long,
        value_enum,
        default_value_t = OutputFormat::Table,
        help = "Output format (table or json)"
    )]
    format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct AddModel {
    model_name: String,
    #[arg(
        long = "param",
        short = 'p',
        help = "Model parameters in key=value format (can be specified multiple times)"
    )]
    param: Vec<String>,
    #[arg(
        long = "info",
        short = 'i',
        help = "Model info in key=value format (can be specified multiple times)"
    )]
    info: Vec<String>,
}

#[derive(Debug, Args)]
pub struct DeleteModel {
    model_id: String,
}

#[derive(Debug, Args)]
pub struct GetModel {
    #[arg(long = "id", help = "ID of the model to retrieve")]
    model_id: Option<String>,
    #[arg(long = "name", help = "Name of the model to retrieve")]
    model_name: Option<String>,
}

#[derive(Debug, Args)]
pub struct GetModelsInfo {
    #[arg(
        long,
        value_enum,
        default_value_t = OutputFormat::Table,
        help = "Output format (table or json)"
    )]
    format: OutputFormat,
    #[arg(
        long,
        default_value = "public_model,upstream_model,updated_at",
        help = "Comma-separated list of columns to display. Valid columns: public_model, upstream_model, credential_name, created_at, updated_at, id, input_cost, output_cost. Default: public_model,upstream_model,updated_at"
    )]
    columns: String,
}

#[derive(Debug, Args)]
pub struct UpdateModel {
    model_id: String,
    #[arg(
        long = "param",
        short = 'p',
        help = "Model parameters in key=value format (can be specified multiple times)"
    )]
    param: Vec<String>,
    #[arg(
        long = "info",
        short = 'i',
        help = "Model info in key=value format (can be specified multiple times)"
    )]
    info: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ImportModels {
    yaml_file: PathBuf,
    #[arg(long, help = "Show what would be imported without making any changes.")]
    dry_run: bool,
    #[arg(
        long = "only-models-matching-regex",
        help = "Only import models where litellm_params.model matches the given regex."
    )]
    model_regex: Option<String>,
    #[arg(
        long = "only-access-groups-matching-regex",
        help = "Only import models where at least one item in model_info.access_groups matches the given regex."
    )]
    access_group_regex: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelsError {
    #[error(transparent)]
    Client(#[from] super::ClientError),
    #[error("the proxy answered HTTP {status}")]
    Http { status: u16 },
    #[error("unexpected response from {path}")]
    Response {
        path: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("unexpected response from {path}: expected a list of models")]
    ExpectedList { path: &'static str },
    #[error("the {field} field has the wrong type")]
    FieldType { field: &'static str },
    #[error("the {field} field is not a valid number")]
    Number {
        field: &'static str,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("the {field} field is not a valid RFC 3339 datetime")]
    DateTime {
        field: &'static str,
        #[source]
        source: chrono::ParseError,
    },
    #[error("the {field} field is outside the supported timestamp range")]
    Timestamp { field: &'static str },
    #[error("the {field} field is too large to price per thousand tokens")]
    Cost { field: &'static str },
    #[error("{flag} is not a valid regex")]
    Regex {
        flag: &'static str,
        #[source]
        source: fancy_regex::Error,
    },
    #[error("{flag} could not be matched against a value")]
    RegexMatch {
        flag: &'static str,
        #[source]
        source: fancy_regex::Error,
    },
    #[error("the {flag} value {value} is not key=value")]
    Pair { flag: &'static str, value: String },
    #[error("{flag} sets {key} more than once")]
    Duplicate { flag: &'static str, key: String },
    #[error("Either --id or --name must be provided")]
    MissingSelector,
    #[error("Exactly one of model_id or model_name must be provided")]
    BothSelectors,
    #[error("{selector} not found")]
    NotFound { selector: String },
    #[error("cannot read {}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse the YAML file")]
    Yaml {
        #[source]
        source: serde_saphyr::Error,
    },
    #[error("YAML file must contain a 'model_list' key with a list of models.")]
    MissingModelList,
    #[error("'model_list' must be a list of model definitions.")]
    ModelListNotList,
    #[error("model {index} is not a mapping")]
    EntryType { index: usize },
    #[error("the {field} field of model {index} is not valid")]
    EntryField { index: usize, field: &'static str },
    #[error("cannot encode the model request")]
    Encode(#[source] serde_json::Error),
    #[error("the model count overflows")]
    Count,
}

pub fn run_models_command(
    command: ModelsCommand,
    client: &Client,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), ModelsError> {
    match command {
        ModelsCommand::List(options) => list::list_models(&options, client, stdout),
        ModelsCommand::Add(options) => add::add_model(&options, client, stdout),
        ModelsCommand::Delete(options) => delete::delete_model(&options, client, stdout),
        ModelsCommand::Get(options) => get::get_model(&options, client, stdout),
        ModelsCommand::Info(options) => info::get_models_info(&options, client, stdout, stderr),
        ModelsCommand::Update(options) => update::update_model(&options, client, stdout),
        ModelsCommand::Import(options) => import::import_models(&options, client, stdout),
    }
}

mod tests;
