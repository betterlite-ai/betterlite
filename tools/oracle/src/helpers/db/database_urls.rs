use std::collections::BTreeMap;

use betterlite_db::database_url_policy::query::QueryValue;
use betterlite_db::database_urls::cli_url::construct_cli_database_url;
use betterlite_db::database_urls::error::DatabaseUrlError;
use betterlite_db::database_urls::password::{
    apply_writer_url_to_env, build_reader_url, build_writer_url,
};
use betterlite_db::database_urls::scheme::{validate_cli_urls, validate_component_schemes};
use betterlite_db::database_urls::settings::{AuthMode, DatabaseUrlSettings, from_env};
use betterlite_db::database_urls::tls::tls_params;
use secrecy::{ExposeSecret as _, SecretString};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    BuildWriterUrl {
        settings: Box<DatabaseUrlSettings>,
        #[serde(default)]
        auth: AuthMode,
    },
    BuildReaderUrl {
        settings: Box<DatabaseUrlSettings>,
        #[serde(default)]
        auth: AuthMode,
    },
    TlsParams {
        settings: Box<DatabaseUrlSettings>,
    },
    FromEnv {
        environment: BTreeMap<String, String>,
    },
    ValidateSchemes {
        settings: Box<DatabaseUrlSettings>,
    },
    ValidateCliUrls {
        environment: BTreeMap<String, String>,
    },
    ApplyWriterUrl {
        settings: Box<DatabaseUrlSettings>,
    },
    ConstructCliUrl {
        environment: BTreeMap<String, String>,
    },
    Batch {
        calls: Vec<Self>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BuildWriterUrl { settings, auth } => {
                let url =
                    build_writer_url(settings, *auth).map_err(|error| helper_error(&error))?;
                Ok(secret_value(url))
            }
            Input::BuildReaderUrl { settings, auth } => {
                let url =
                    build_reader_url(settings, *auth).map_err(|error| helper_error(&error))?;
                Ok(secret_value(url))
            }
            Input::TlsParams { settings } => Ok(tls_params_value(settings)),
            Input::FromEnv { environment } => {
                let settings = from_env(environment)
                    .map_err(|source| helper_error(&DatabaseUrlError::Settings { source }))?;
                Ok(settings_value(&settings))
            }
            Input::ValidateSchemes { settings } => {
                validate_component_schemes(settings).map_err(|error| helper_error(&error))?;
                Ok(serde_json::Value::Null)
            }
            Input::ValidateCliUrls { environment } => {
                validate_cli_urls(environment).map_err(|error| helper_error(&error))?;
                Ok(serde_json::Value::Null)
            }
            Input::ApplyWriterUrl { settings } => writer_update_value(settings),
            Input::ConstructCliUrl { environment } => {
                let url = construct_cli_database_url(environment)
                    .map_err(|error| helper_error(&error))?;
                Ok(secret_value(url))
            }
            Input::Batch { calls } => Ok(serde_json::Value::Array(
                calls.iter().map(scenario_value).collect(),
            )),
        }
    }
}

fn secret_value(value: Option<SecretString>) -> serde_json::Value {
    value.map_or(serde_json::Value::Null, |secret| {
        serde_json::Value::String(secret.expose_secret().to_owned())
    })
}

fn scenario_value(input: &Input) -> serde_json::Value {
    match Op::run(input) {
        Ok(value) => serde_json::json!({"ok": true, "value": value}),
        Err(_error) => serde_json::json!({"ok": false}),
    }
}

fn tls_params_value(settings: &DatabaseUrlSettings) -> serde_json::Value {
    let pairs: Vec<serde_json::Value> = tls_params(settings)
        .iter()
        .map(|(key, value)| serde_json::json!([key, query_value_json(value)]))
        .collect();
    serde_json::Value::Array(pairs)
}

fn query_value_json(value: &QueryValue) -> serde_json::Value {
    match value {
        QueryValue::Integer(number) => serde_json::json!(number),
        QueryValue::Float(number) => serde_json::json!(number),
        QueryValue::Text(text) => serde_json::Value::String(text.clone()),
    }
}

fn settings_value(settings: &DatabaseUrlSettings) -> serde_json::Value {
    serde_json::json!({
        "database_url": settings.database_url.as_ref().map(SecretString::expose_secret),
        "direct_url": settings.direct_url.as_ref().map(SecretString::expose_secret),
        "database_host": &settings.database_host,
        "database_port": &settings.database_port,
        "database_user": &settings.database_user,
        "database_name": &settings.database_name,
        "database_schema": &settings.database_schema,
        "database_password_set": settings.database_password.is_some(),
        "database_url_read_replica": settings
            .database_url_read_replica
            .as_ref()
            .map(SecretString::expose_secret),
        "database_host_read_replica": &settings.database_host_read_replica,
        "database_port_read_replica": &settings.database_port_read_replica,
        "database_user_read_replica": &settings.database_user_read_replica,
        "database_name_read_replica": &settings.database_name_read_replica,
        "database_schema_read_replica": &settings.database_schema_read_replica,
        "database_password_read_replica_set": settings.database_password_read_replica.is_some(),
        "database_sslmode": &settings.database_sslmode,
        "database_sslrootcert": &settings.database_sslrootcert,
        "max_idle_connection_lifetime": settings.max_idle_connection_lifetime,
        "disable_prepared_statements": settings.disable_prepared_statements,
        "iam_token_db_auth": settings.iam_token_db_auth,
        "azure_postgresql_auth": settings.azure_postgresql_auth,
    })
}

fn writer_update_value(settings: &DatabaseUrlSettings) -> HelperResult {
    let result = apply_writer_url_to_env(settings, AuthMode::Password)
        .map_err(|error| helper_error(&error))?;
    let updates: Vec<serde_json::Value> = result
        .updates
        .iter()
        .map(|update| serde_json::json!([&update.name, update.value.expose_secret()]))
        .collect();
    Ok(serde_json::json!({"replaced": result.replaced, "updates": updates}))
}

fn helper_error(error: &DatabaseUrlError) -> Box<HelperError> {
    let class = match error {
        DatabaseUrlError::Url { .. } => "ValueError",
        DatabaseUrlError::Settings { .. } => "ValidationError",
        DatabaseUrlError::UnsupportedScheme { .. } => "RuntimeError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
