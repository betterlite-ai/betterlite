use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_server::server::run;
use clap::Parser;
use std::collections::BTreeMap;
use std::fs;
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
    let source = match fs::read_to_string(&cli.config) {
        Ok(source) => source,
        Err(error) => {
            tracing::error!(path = %cli.config.display(), error = ?error, "failed to read configuration");
            return ExitCode::FAILURE;
        }
    };
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let config = match loader.load(&source) {
        Ok(config) => config,
        Err(error) => {
            log_config_error(&error, &cli.config);
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
    match runtime.block_on(run(config, &cli.host, cli.port)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server stopped with an error");
            ExitCode::FAILURE
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
