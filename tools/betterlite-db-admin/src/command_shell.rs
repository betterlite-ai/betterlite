use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::chatgpt_token_store::home::HomeEnvironment;
use betterlite_config::sections::includes::{LoadError, load_file};
use betterlite_config::types::Config;
use betterlite_db::database_urls::error::DatabaseUrlError;
use betterlite_db::database_urls::password::apply_writer_url_to_env;
use betterlite_db::database_urls::settings::AuthMode;
use clap::Parser as _;
use clap::error::ErrorKind;
use secrecy::{ExposeSecret as _, SecretString};

use crate::commands::{Cli, DbAdminCommand, DbAdminError, DbAdminOutcome, Operation};

const EMPTY_CONFIG: &str = "";

pub struct AdminEnvironment {
    pub variables: BTreeMap<String, String>,
    pub home: Option<PathBuf>,
}

impl AdminEnvironment {
    pub fn from_os<I>(
        variables: I,
        home: Option<PathBuf>,
        diagnostics: &mut dyn Diagnostics,
    ) -> Self
    where
        I: IntoIterator<Item = (OsString, OsString)>,
    {
        let mut decoded = BTreeMap::new();
        for (raw_name, raw_value) in variables {
            match (raw_name.into_string(), raw_value.into_string()) {
                (Ok(name), Ok(value)) => {
                    decoded.insert(name, value);
                }
                (Ok(name), Err(_)) => diagnostics.error(&format!(
                    "environment variable {name} is not UTF-8 and is ignored"
                )),
                (Err(_), _) => {
                    diagnostics.error("an environment variable name is not UTF-8 and is ignored");
                }
            }
        }
        Self {
            variables: decoded,
            home,
        }
    }
}

pub trait Diagnostics: Send {
    fn output(&mut self, line: &str);
    fn error(&mut self, line: &str);
}

pub struct Streams<'stream> {
    pub output: &'stream mut (dyn Write + Send),
    pub errors: &'stream mut (dyn Write + Send),
}

impl Diagnostics for Streams<'_> {
    fn output(&mut self, line: &str) {
        drop(writeln!(self.output, "{line}"));
    }

    fn error(&mut self, line: &str) {
        drop(writeln!(self.errors, "{line}"));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Success,
    Failure,
    Usage,
    Unrecoverable,
}

impl ExitStatus {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Usage | Self::Unrecoverable => 2,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum PrepareError {
    #[error("the configuration file could not be loaded: {source}")]
    ConfigFile {
        #[source]
        source: LoadError,
    },
    #[error("the configuration could not be resolved: {source}")]
    Config {
        #[source]
        source: ConfigError,
    },
    #[error("the database URL could not be resolved: {source}")]
    Url {
        #[source]
        source: DatabaseUrlError,
    },
    #[error(
        "DATABASE_URL is not set and could not be assembled from the DATABASE_* env vars \u{2014} cannot run migrations."
    )]
    MissingUrl,
}

// the writer URL stays a SecretString in database_url; environment is the operator's variables as read
pub struct ResolvedConfig {
    pub settings: Config,
    pub environment: BTreeMap<String, String>,
    pub database_url: SecretString,
}

pub type DbAdminFuture<'service> =
    Pin<Box<dyn Future<Output = Result<DbAdminOutcome, DbAdminError>> + Send + 'service>>;

pub trait DbAdminService: Send + Sync {
    fn run(&self, command: DbAdminCommand, config: ResolvedConfig) -> DbAdminFuture<'_>;
}

#[derive(Default)]
pub struct DbAdminBackend {
    pub services: BTreeMap<Operation, Arc<dyn DbAdminService>>,
}

impl DbAdminBackend {
    pub async fn run(
        &self,
        command: DbAdminCommand,
        config: ResolvedConfig,
    ) -> Result<DbAdminOutcome, DbAdminError> {
        let operation = command.operation();
        let Some(service) = self.services.get(&operation) else {
            return Err(DbAdminError::Unregistered {
                operation: <&'static str>::from(operation),
            });
        };
        service.run(command, config).await
    }
}

pub async fn run<I>(
    argv: I,
    environment: AdminEnvironment,
    backend: &DbAdminBackend,
    diagnostics: &mut dyn Diagnostics,
) -> ExitStatus
where
    I: IntoIterator<Item = OsString>,
{
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(error) => return parse_exit(&error, diagnostics),
    };
    match prepare(cli.config.as_deref(), cli.operation, &environment) {
        Ok((command, config)) => {
            if let DbAdminCommand::Migrate(options) = command {
                diagnostics.error(&format!(
                    "Starting migration job (use_migrate={}, use_v2_resolver={})",
                    options.use_migrate, options.use_v2_resolver
                ));
            }
            outcome_exit(backend.run(command, config).await, diagnostics)
        }
        Err(error) => {
            diagnostics.error(&error.to_string());
            ExitStatus::Failure
        }
    }
}

fn prepare(
    config_path: Option<&Path>,
    operation: Operation,
    environment: &AdminEnvironment,
) -> Result<(DbAdminCommand, ResolvedConfig), PrepareError> {
    let settings = load_config(config_path, environment)?;
    let endpoints = &settings.sections.database_urls.endpoints;
    let auth = if endpoints.iam_token_db_auth || endpoints.azure_postgresql_auth {
        AuthMode::Token
    } else {
        AuthMode::Password
    };
    let writer =
        apply_writer_url_to_env(endpoints, auth).map_err(|source| PrepareError::Url { source })?;
    let assembled = writer.updates.into_iter().next().map(|update| update.value);
    let pinned = endpoints
        .database_url
        .clone()
        .filter(|url| !url.expose_secret().is_empty());
    let database_url = assembled.or(pinned).ok_or(PrepareError::MissingUrl)?;
    let command = operation.command(&environment.variables);
    Ok((
        command,
        ResolvedConfig {
            settings,
            environment: environment.variables.clone(),
            database_url,
        },
    ))
}

fn load_config(
    path: Option<&Path>,
    environment: &AdminEnvironment,
) -> Result<Config, PrepareError> {
    let loader = ConfigLoader {
        lookup: HomeEnvironment {
            variables: environment.variables.clone(),
            home: environment.home.clone(),
        },
    };
    path.map_or_else(
        || {
            loader
                .load(EMPTY_CONFIG)
                .map_err(|source| PrepareError::Config { source })
        },
        |file| load_file(&loader, file).map_err(|source| PrepareError::ConfigFile { source }),
    )
}

fn parse_exit(error: &clap::Error, diagnostics: &mut dyn Diagnostics) -> ExitStatus {
    if matches!(
        error.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        diagnostics.output(&error.to_string());
        return ExitStatus::Success;
    }
    diagnostics.error(&error.to_string());
    ExitStatus::Usage
}

fn outcome_exit(
    outcome: Result<DbAdminOutcome, DbAdminError>,
    diagnostics: &mut dyn Diagnostics,
) -> ExitStatus {
    match outcome {
        Ok(DbAdminOutcome::Completed) => {
            diagnostics.error("Migration job completed successfully.");
            ExitStatus::Success
        }
        Ok(DbAdminOutcome::RetriesExhausted | DbAdminOutcome::IndexJobFailed) => {
            diagnostics.error("Migration job failed after retries.");
            ExitStatus::Failure
        }
        Ok(DbAdminOutcome::Views { lines }) => {
            for line in &lines {
                diagnostics.output(line);
            }
            ExitStatus::Success
        }
        Err(DbAdminError::Views { lines, source }) => {
            for line in &lines {
                diagnostics.output(line);
            }
            diagnostics.error(&source.to_string());
            ExitStatus::Failure
        }
        Err(error @ DbAdminError::Unrecoverable { .. }) => {
            diagnostics.error(&format!("Database migration cannot proceed: {error}"));
            ExitStatus::Unrecoverable
        }
        Err(error) => {
            diagnostics.error(&error.to_string());
            ExitStatus::Failure
        }
    }
}

mod tests;
