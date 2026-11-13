use super::ServerError;
use super::chat_setup::chat_executor;
use super::filter::{ComponentRoutes, with_component_filter};
use super::run_with;
use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::chatgpt_token_store::home::HomeEnvironment;
use betterlite_config::sections::includes::{IncludeError, LoadError, load_file};
use betterlite_config::types::Config;
use betterlite_openai::chat::execution::ChatExecutor;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysRng};
use std::collections::BTreeMap;
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tokio::runtime::Builder;

// a component binary owns only its CLI defaults, its path lists and main; the startup runs here once
pub fn serve_component(
    config_path: PathBuf,
    host: String,
    port: u16,
    routes: ComponentRoutes,
) -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .flatten_event(true)
        .with_target(false)
        .with_writer(io::stderr)
        .init();
    let Some((config, chat)) = load_runtime(&config_path) else {
        return ExitCode::FAILURE;
    };
    let runtime = match Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(error = ?error, "failed to build runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(serve(config, chat, &host, port, routes)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "server startup failed");
            ExitCode::from(2)
        }
    }
}

fn load_runtime(config_path: &Path) -> Option<(Config, ChatExecutor)> {
    let environment = process_environment();
    let config = load_config(config_path, environment.clone())?;
    let random = match StdRng::try_from_rng(&mut SysRng) {
        Ok(random) => random,
        Err(error) => {
            tracing::error!(error = %error, "failed to seed the deployment picker from the OS");
            return None;
        }
    };
    match chat_executor(&config, &environment, random) {
        Ok(executor) => Some((config, executor)),
        Err(error) => {
            tracing::error!(error = %error, "failed to set up chat completions");
            None
        }
    }
}

async fn serve(
    config: Config,
    chat: ChatExecutor,
    host: &str,
    port: u16,
    routes: ComponentRoutes,
) -> Result<(), ServerError> {
    run_with(config, chat, host, port, |application| {
        with_component_filter(application, routes)
    })
    .await
}

#[expect(
    clippy::disallowed_methods,
    reason = "the component binary is where the process environment enters, and it hands every variable to the config crate"
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
        lookup: HomeEnvironment {
            variables: environment,
            home: env::home_dir(),
        },
    };
    match load_file(&loader, location) {
        Ok(config) => Some(config),
        Err(error) => {
            match error {
                LoadError::Config { source } => {
                    log_config_error(&source, location);
                }
                LoadError::Includes { source } => {
                    if let IncludeError::Yaml { path, .. } = &source {
                        tracing::error!(category = "yaml", include = %path.display(), location = %location.display(), "failed to read or resolve configuration");
                    } else {
                        tracing::error!(path = %location.display(), error = %source, "failed to read or resolve configuration");
                    }
                }
                LoadError::Serialize { source } => {
                    tracing::error!(
                        path = %location.display(),
                        error = %source,
                        "failed to serialize resolved configuration"
                    );
                }
            }
            None
        }
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
        | ConfigError::InvalidScalar { .. }
        | ConfigError::Environment { .. } => {
            tracing::error!(
                category = error.category(),
                location = %location.display(),
                "failed to load configuration"
            );
        }
    }
}
