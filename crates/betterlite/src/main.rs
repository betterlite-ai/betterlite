use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::types::Config;
use betterlite_server::server::chat_setup::chat_executor;
use betterlite_server::server::run;
use clap::Parser;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use std::collections::BTreeMap;
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tokio::runtime::Builder;
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(short, long)]
    config: PathBuf,
    #[arg(long, env = "HOST", default_value = "0.0.0.0")]
    host: String,
    #[arg(long, env = "PORT", default_value_t = 4000)]
    port: u16,
}
fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let cli = Cli::parse();
    let environment = process_environment();
    let Some(config) = load_config(&cli.config, environment.clone()) else {
        return ExitCode::FAILURE;
    };
    let random = match StdRng::try_from_rng(&mut SysRng) {
        Ok(random) => random,
        Err(error) => {
            tracing::error!(error = %error, "failed to seed the deployment picker from the OS");
            return ExitCode::FAILURE;
        }
    };
    let chat = match chat_executor(&config, &environment, random) {
        Ok(executor) => executor,
        Err(error) => {
            tracing::error!(error = %error, "failed to set up chat completions");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(error = ?error, "failed to build runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(config, chat, &cli.host, cli.port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server stopped with an error");
            ExitCode::FAILURE
        }
    }
}
#[expect(
    clippy::disallowed_methods,
    reason = "the binary is where the process environment enters, and it hands every variable to the config crate"
)]
fn process_environment() -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for (raw_name, raw_value) in env::vars_os() {
        match (raw_name.into_string(), raw_value.into_string()) {
            (Ok(name), Ok(value)) => {
                environment.insert(name, value);
            }
            (Ok(name), Err(_)) => {
                tracing::warn!(
                    variable = name,
                    "environment variable is not UTF-8 and is ignored"
                );
            }
            (Err(_), _) => {
                tracing::warn!("an environment variable name is not UTF-8 and is ignored");
            }
        }
    }
    environment
}
fn load_config(location: &Path, environment: BTreeMap<String, String>) -> Option<Config> {
    let loader = ConfigLoader {
        lookup: environment,
    };
    match loader.load_file(location) {
        Ok(config) => Some(config),
        Err(error) => {
            match error {
                betterlite_config::sections::includes::LoadError::Config { source } => {
                    log_config_error(&source, location);
                }
                error => {
                    tracing::error!(
                        path = %location.display(),
                        error = ?error,
                        "failed to read or resolve configuration"
                    );
                }
            }
            None
        }
    }
}
const fn config_error_category(error: &ConfigError) -> &'static str {
    match error {
        ConfigError::Yaml { .. } => "yaml",
        ConfigError::Unsupported { .. } => "unsupported_key",
        ConfigError::MissingEnvironment { .. } => "missing_environment",
        ConfigError::InvalidFloat { .. } => "invalid_float",
        ConfigError::InvalidInteger { .. } => "invalid_integer",
        ConfigError::InvalidScalar { .. } => "invalid_scalar",
    }
}
fn log_config_error(error: &ConfigError, location: &Path) {
    match error {
        ConfigError::Unsupported { paths } => {
            tracing::error!(
                category = "unsupported_key",
                paths = ?paths,
                location = %location.display(),
                "failed to load configuration"
            );
        }
        ConfigError::MissingEnvironment { variable, path } => {
            tracing::error!(
                category = "missing_environment",
                variable,
                path,
                location = %location.display(),
                "failed to load configuration"
            );
        }
        ConfigError::Yaml { .. }
        | ConfigError::InvalidFloat { .. }
        | ConfigError::InvalidInteger { .. }
        | ConfigError::InvalidScalar { .. } => {
            tracing::error!(
                category = config_error_category(error),
                location = %location.display(),
                "failed to load configuration"
            );
        }
    }
}
