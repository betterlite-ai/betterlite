use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::chatgpt_token_store::home::HomeEnvironment;
use crate::sections::includes::{IncludeError, LoadError, load_file};
use crate::types::Config;
use std::collections::BTreeMap;
use std::env;
use std::path::Path;

#[expect(
    clippy::disallowed_methods,
    reason = "every binary reads the process environment through this one function and hands each variable to the config crate"
)]
#[must_use]
pub fn process_environment() -> BTreeMap<String, String> {
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

#[must_use]
pub fn load_config(location: &Path, environment: BTreeMap<String, String>) -> Option<Config> {
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
