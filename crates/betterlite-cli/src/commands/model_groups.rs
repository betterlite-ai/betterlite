use std::io::Write;

use clap::{Args, Subcommand};
use http::Method;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;

use super::tables::{OutputFormat, write_json_response, write_table};
use super::{Client, ClientError, Payload, Request};

const INFO_PATH: &str = "/model_group/info";

#[derive(Debug, Subcommand)]
#[command(about = "Inspect model groups your key can access on the proxy")]
pub enum ModelGroupsCommand {
    List(ListModelGroups),
}

#[derive(Debug, Args)]
pub struct ListModelGroups {
    #[arg(
        long,
        value_enum,
        default_value_t = OutputFormat::Table,
        help = "Output format (table or json)"
    )]
    format: OutputFormat,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelGroupsError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("the proxy answered HTTP {status}")]
    Http { status: u16 },
    #[error("unexpected response from {path}")]
    Response {
        path: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Unexpected response from /model_group/info: expected a list, got {kind}")]
    UnexpectedType { kind: &'static str },
}

#[derive(Debug, Deserialize)]
struct ModelGroupInfoResponse {
    data: Box<RawValue>,
}

#[derive(Debug, Deserialize)]
struct ModelGroupRow {
    #[serde(default)]
    model_group: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    input_cost_per_token: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    output_cost_per_token: Option<Decimal>,
}

pub fn run_model_groups_command(
    command: ModelGroupsCommand,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelGroupsError> {
    match command {
        ModelGroupsCommand::List(options) => list_model_groups(&options, client, stdout),
    }
}

fn list_model_groups(
    options: &ListModelGroups,
    client: &Client,
    stdout: &mut dyn Write,
) -> Result<(), ModelGroupsError> {
    let request = Request {
        method: Method::GET,
        url: client.resolve(INFO_PATH)?,
        headers: Vec::new(),
        payload: Payload::Empty,
    };
    let response = client.send(request)?;
    if response.status >= 400 {
        return Err(ModelGroupsError::Http {
            status: response.status,
        });
    }
    let view: ModelGroupInfoResponse =
        serde_json::from_slice(&response.body).map_err(|source| ModelGroupsError::Response {
            path: INFO_PATH,
            source,
        })?;
    let data = view.data.get();
    if !data.trim_start().starts_with('[') {
        return Err(ModelGroupsError::UnexpectedType {
            kind: json_kind(data),
        });
    }
    render_model_groups(data, options.format, stdout)
}

fn render_model_groups(
    data: &str,
    format: OutputFormat,
    stdout: &mut dyn Write,
) -> Result<(), ModelGroupsError> {
    if format == OutputFormat::Json {
        return write_json_response(data.as_bytes(), stdout).map_err(|source| {
            ModelGroupsError::Response {
                path: INFO_PATH,
                source,
            }
        });
    }
    let groups: Vec<ModelGroupRow> =
        serde_json::from_str(data).map_err(|source| ModelGroupsError::Response {
            path: INFO_PATH,
            source,
        })?;
    write_table(
        "Accessible Model Groups",
        &["Model", "Mode", "Input $/token", "Output $/token"],
        model_group_rows(groups),
        stdout,
    );
    Ok(())
}

fn model_group_rows(groups: Vec<ModelGroupRow>) -> Vec<Vec<String>> {
    groups
        .into_iter()
        .map(|group| {
            vec![
                group.model_group.unwrap_or_default(),
                group.mode.unwrap_or_else(|| String::from("chat")),
                group
                    .input_cost_per_token
                    .map(|cost| cost.to_string())
                    .unwrap_or_default(),
                group
                    .output_cost_per_token
                    .map(|cost| cost.to_string())
                    .unwrap_or_default(),
            ]
        })
        .collect()
}

fn json_kind(raw: &str) -> &'static str {
    match raw.trim_start().as_bytes().first() {
        Some(b'{') => "object",
        Some(b'[') => "array",
        Some(b'"') => "string",
        Some(b't' | b'f') => "bool",
        Some(b'n') => "null",
        Some(_) | None => "number",
    }
}

mod tests;
