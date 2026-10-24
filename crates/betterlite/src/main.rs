use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::types::Config;
use betterlite_server::server::run;
use clap::Parser;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use tokio::runtime::Builder;
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[arg(long)]
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
    let Ok(config) = load_config(&cli.config) else {
        return ExitCode::FAILURE;
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
fn load_config(path: &PathBuf) -> Result<Config, ()> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            tracing::error!(path = %path.display(), error = ?error, "failed to read configuration");
            return Err(());
        }
    };
    // The process environment needs a disallowed std::env call, so os.environ/ references fail until the binary may pass it.
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    match loader.load(&source) {
        Ok(config) => Ok(config),
        Err(error) => {
            tracing::error!(
                category = config_error_category(&error),
                location = %path.display(),
                "failed to load configuration"
            );
            Err(())
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
