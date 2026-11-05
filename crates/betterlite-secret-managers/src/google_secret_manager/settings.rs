use std::{collections::BTreeMap, time::Duration};

use super::GoogleSecretManagerError;

const PROJECT_ID_VARIABLE: &str = "GOOGLE_SECRET_MANAGER_PROJECT_ID";
const REFRESH_INTERVAL_VARIABLE: &str = "GOOGLE_SECRET_MANAGER_REFRESH_INTERVAL";
const ALWAYS_READ_VARIABLE: &str = "GOOGLE_SECRET_MANAGER_ALWAYS_READ_SECRET_MANAGER";
const DEFAULT_REFRESH_VARIABLE: &str = "SECRET_MANAGER_REFRESH_INTERVAL";
const DEFAULT_REFRESH_SECONDS: u64 = 86_400;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    pub refresh_interval: Option<u64>,
    pub always_read_secret_manager: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub project_id: String,
    pub refresh_interval: Duration,
    pub always_read_secret_manager: bool,
}

impl Settings {
    pub fn resolve(
        environment: &BTreeMap<String, String>,
        options: Options,
    ) -> Result<Self, GoogleSecretManagerError> {
        let project_id = environment
            .get(PROJECT_ID_VARIABLE)
            .filter(|project_id| !project_id.is_empty())
            .ok_or(GoogleSecretManagerError::MissingProjectId)?
            .clone();
        let refresh_seconds = match read_seconds(environment, REFRESH_INTERVAL_VARIABLE)? {
            Some(seconds) => seconds,
            None => match options.refresh_interval {
                Some(seconds) => seconds,
                None => read_seconds(environment, DEFAULT_REFRESH_VARIABLE)?
                    .unwrap_or(DEFAULT_REFRESH_SECONDS),
            },
        };
        let always_read_from_environment = environment
            .get(ALWAYS_READ_VARIABLE)
            .map(|text| betterlite_env::flag(ALWAYS_READ_VARIABLE, text))
            .transpose()
            .map_err(GoogleSecretManagerError::Environment)?
            .flatten()
            .unwrap_or(false);
        Ok(Self {
            project_id,
            refresh_interval: Duration::from_secs(refresh_seconds),
            always_read_secret_manager: always_read_from_environment
                || options.always_read_secret_manager.unwrap_or(false),
        })
    }
}

fn read_seconds(
    environment: &BTreeMap<String, String>,
    name: &str,
) -> Result<Option<u64>, GoogleSecretManagerError> {
    environment
        .get(name)
        .map(|text| betterlite_env::number::<u64>(name, text))
        .transpose()
        .map(Option::flatten)
        .map_err(GoogleSecretManagerError::Environment)
}

mod tests;
