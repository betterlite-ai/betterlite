use std::io::{self, Write};
use std::path::Path;
use std::process as std_process;

use betterlite_config::loader::{ConfigLoader, EnvironmentLookup};
use betterlite_config::sections::includes::{LoadError, load_file};
use betterlite_config::sections::settings::JsonValue;
use betterlite_http::health_probe::{HealthTransport, TransportError};
use serde::Serialize as _;
use serde_json::ser::PrettyFormatter;

const TEST_DEFAULT_MODEL: &str = "gpt-3.5-turbo";
const INDENT: &[u8] = b"    ";

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("the health response is not JSON: {source}")]
    NotJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the health request failed: {source}")]
    Transport {
        #[from]
        source: TransportError,
    },
    #[error("--validate_config requires --config <path>")]
    ConfigRequired,
    #[error("config validation failed: {source}")]
    ConfigInvalid {
        #[source]
        source: LoadError,
    },
    #[error("Invalid test value")]
    InvalidTestValue,
}

pub fn version_command(
    package_version: &str,
    writer: &mut dyn Write,
) -> Result<(), DiagnosticError> {
    write!(writer, "\nLiteLLM: Current Version = {package_version}\n\n")?;
    Ok(())
}

pub fn health_command(
    host: &str,
    port: u16,
    transport: &dyn HealthTransport,
    writer: &mut dyn Write,
) -> Result<(), DiagnosticError> {
    writeln!(writer, "\nLiteLLM: Health Testing models in config")?;
    let body = transport.get(&format!("http://{host}:{port}/health"))?;
    let value: JsonValue =
        serde_json::from_str(&body).map_err(|source| DiagnosticError::NotJson { source })?;
    let mut rendered = Vec::new();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut rendered, PrettyFormatter::with_indent(INDENT));
    value
        .serialize(&mut serializer)
        .map_err(|source| DiagnosticError::NotJson { source })?;
    writer.write_all(&rendered)?;
    writeln!(writer)?;
    Ok(())
}

pub fn config_validation_command<L>(
    config: Option<&Path>,
    loader: &ConfigLoader<L>,
    writer: &mut dyn Write,
    err_writer: &mut dyn Write,
) -> Result<(), DiagnosticError>
where
    L: EnvironmentLookup,
{
    let Some(path) = config else {
        return Err(DiagnosticError::ConfigRequired);
    };
    match load_file(loader, path) {
        Ok(resolved) => {
            // Router.get_model_list also lists every deployment of a visible alias's group under the alias name
            let aliased: usize = resolved
                .sections
                .deployment_catalog
                .model_group_alias
                .values()
                .filter(|alias| !alias.hidden)
                .map(|alias| {
                    resolved
                        .model_list
                        .iter()
                        .filter(|deployment| deployment.model_name == alias.model)
                        .count()
                })
                .sum();
            writeln!(
                writer,
                "LiteLLM: config OK ({} models)",
                resolved.model_list.len().saturating_add(aliased)
            )?;
            Ok(())
        }
        Err(source) => {
            writeln!(err_writer, "LiteLLM: config validation failed: {source}")?;
            Err(DiagnosticError::ConfigInvalid { source })
        }
    }
}

pub fn test_command(
    _host: &str,
    _port: u16,
    model: Option<&str>,
    writer: &mut dyn Write,
) -> Result<(), DiagnosticError> {
    let request_model = model
        .filter(|name| !name.is_empty())
        .unwrap_or(TEST_DEFAULT_MODEL);
    writeln!(
        writer,
        "\nLiteLLM: Making a test ChatCompletions request to your proxy. Model={request_model}"
    )?;
    Err(DiagnosticError::InvalidTestValue)
}

pub trait ProcessLauncher {
    fn spawn_silent(&self, program: &str, args: &[&str]) -> Result<(), io::Error>;
}

pub struct SilentLauncher;

impl ProcessLauncher for SilentLauncher {
    fn spawn_silent(&self, program: &str, args: &[&str]) -> Result<(), io::Error> {
        std_process::Command::new(program)
            .args(args)
            .stdin(std_process::Stdio::null())
            .stdout(std_process::Stdio::null())
            .stderr(std_process::Stdio::null())
            .spawn()
            .map(drop)
    }
}

pub fn ollama_serve(
    launcher: &dyn ProcessLauncher,
    writer: &mut dyn Write,
) -> Result<(), DiagnosticError> {
    match launcher.spawn_silent("ollama", &["serve"]) {
        Ok(()) => Ok(()),
        Err(error) => {
            write!(
                writer,
                "\n                LiteLLM Warning: proxy started with `ollama` model\n`ollama serve` failed with Exception{error}. \nEnsure you run `ollama serve`\n            \n"
            )?;
            Ok(())
        }
    }
}

#[must_use]
pub fn needs_ollama(model: Option<&str>, api_base: Option<&str>) -> bool {
    if api_base.is_some() {
        return false;
    }
    model.is_some_and(|name| name.contains("ollama"))
}

mod tests;
