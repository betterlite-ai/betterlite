use betterlite_db::database_url_policy::flag::{FlagValue, str_to_bool, token_auth_flag_enabled};
use betterlite_db::database_url_policy::policy::{
    ConfiguredSeconds, DatabaseTimeoutSettings, PoolTimeouts, TimeoutError,
    build_db_connection_url_params, pg_options_with_timeouts, unsupported_db_scheme,
    unsupported_db_scheme_message,
};
use betterlite_db::database_url_policy::query::{
    AppendQueryParam, QueryValue, add_missing_query_params, append_query_params,
    connection_params_from_url, idle_lifetime_params, reader_shareable_params,
    token_refresh_params_from_url, url_query_value, with_query_value,
};
pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::IdleLifetimeParams { configured } => {
                Ok(params_value(&idle_lifetime_params(configured.clone())))
            }
            Input::AddMissingQueryParams { url, params } => {
                run_add_missing_query_params(url, params)
            }
            Input::ReaderShareableParams { params } => {
                Ok(params_value(&reader_shareable_params(params)))
            }
            Input::ConnectionParamsFromUrl { url } => {
                value_error(connection_params_from_url(url), |params| {
                    params_value(&params)
                })
            }
            Input::TokenRefreshParamsFromUrl { url } => {
                value_error(token_refresh_params_from_url(url), |params| {
                    params_value(&params)
                })
            }
            Input::UnsupportedDbScheme { database_url } => {
                value_error(unsupported_db_scheme(database_url), |scheme| {
                    scheme.map_or(serde_json::Value::Null, serde_json::Value::String)
                })
            }
            Input::UnsupportedDbSchemeMessage { env_var, scheme } => Ok(serde_json::Value::String(
                unsupported_db_scheme_message(env_var, scheme),
            )),
            Input::BuildDbConnectionUrlParams { .. } => Ok(run_build_connection_params(input)),
            Input::PgOptionsWithTimeouts { .. } => run_pg_options_with_timeouts(input),
            Input::UrlQueryValue { url, key } => run_url_query_value(url.as_deref(), key),
            Input::WithQueryValue { url, key, value } => run_with_query_value(url, key, value),
            Input::AppendQueryParams { url, params } => {
                run_append_query_params(url.as_deref(), params)
            }
            Input::TokenAuthFlagEnabled { value, env_var } => value_error(
                token_auth_flag_enabled(value.as_ref(), env_var),
                serde_json::Value::Bool,
            ),
            Input::StrToBool { value } => {
                Ok(serde_json::Value::Bool(str_to_bool(value.as_deref())))
            }
        }
    }
}

fn run_add_missing_query_params(
    url: &str,
    params: &[(String, QueryValue)],
) -> crate::helpers::HelperResult {
    let updated = add_missing_query_params(url, params)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(serde_json::Value::String(updated))
}

fn run_url_query_value(url: Option<&str>, key: &str) -> crate::helpers::HelperResult {
    let value =
        url_query_value(url, key).map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(serde_json::Value::String(value))
}

fn run_with_query_value(url: &str, key: &str, value: &str) -> crate::helpers::HelperResult {
    let updated = with_query_value(url, key, value)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(serde_json::Value::String(updated))
}

fn run_append_query_params(
    url: Option<&str>,
    params: &[(String, AppendQueryParam)],
) -> crate::helpers::HelperResult {
    let outcome = append_query_params(url, params)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(serde_json::json!({ "url": outcome.url, "warning": outcome.warning }))
}

fn value_error<T>(
    result: Result<T, impl std::fmt::Display>,
    to_value: impl FnOnce(T) -> serde_json::Value,
) -> crate::helpers::HelperResult {
    match result {
        Ok(value) => Ok(to_value(value)),
        Err(error) => Err(helper_error("ValueError", error.to_string())),
    }
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}

fn run_build_connection_params(input: &Input) -> serde_json::Value {
    let Input::BuildDbConnectionUrlParams {
        connection_limit,
        pool_timeout,
        connect_timeout,
        socket_timeout,
        disable_prepared_statements,
        extra_params,
    } = input
    else {
        return serde_json::Value::Null;
    };
    params_value(&build_db_connection_url_params(
        *connection_limit,
        PoolTimeouts {
            pool_timeout: pool_timeout.clone(),
            connect_timeout: connect_timeout.clone(),
            socket_timeout: socket_timeout.clone(),
        },
        *disable_prepared_statements,
        extra_params,
    ))
}

fn params_value(params: &[(String, QueryValue)]) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = params
        .iter()
        .map(|(key, value)| serde_json::json!([key, query_value_json(value)]))
        .collect();
    serde_json::Value::Array(entries)
}

fn query_value_json(value: &QueryValue) -> serde_json::Value {
    match value {
        QueryValue::Integer(number) => serde_json::json!(number),
        QueryValue::Float(number) => serde_json::json!(number),
        QueryValue::Text(text) => serde_json::Value::String(text.clone()),
    }
}

fn run_pg_options_with_timeouts(input: &Input) -> crate::helpers::HelperResult {
    let Input::PgOptionsWithTimeouts {
        existing_options,
        statement_timeout,
        lock_timeout,
    } = input
    else {
        return Ok(serde_json::Value::Null);
    };
    let settings =
        DatabaseTimeoutSettings::validate(statement_timeout.clone(), lock_timeout.clone())
            .map_err(|error| timeout_error(&error))?;
    let options = pg_options_with_timeouts(
        existing_options,
        settings.database_statement_timeout,
        settings.database_lock_timeout,
    )
    .map_err(|error| timeout_error(&error))?;
    Ok(serde_json::Value::String(options))
}

fn timeout_error(error: &TimeoutError) -> Box<crate::helpers::HelperError> {
    let class = match error {
        TimeoutError::NotNumeric { .. } => "ValidationError",
        TimeoutError::NotANumber => "ValueError",
        TimeoutError::Infinite => "OverflowError",
    };
    helper_error(class, error.to_string())
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    IdleLifetimeParams { configured: Option<QueryValue> },
    AddMissingQueryParams { url: String, params: Vec<(String, QueryValue)> },
    ReaderShareableParams { params: Vec<(String, QueryValue)> },
    ConnectionParamsFromUrl { url: String },
    TokenRefreshParamsFromUrl { url: String },
    UnsupportedDbScheme { database_url: String },
    UnsupportedDbSchemeMessage { env_var: String, scheme: String },
    BuildDbConnectionUrlParams {
        connection_limit: i64,
        pool_timeout: Option<QueryValue>,
        connect_timeout: Option<QueryValue>,
        socket_timeout: Option<QueryValue>,
        disable_prepared_statements: bool,
        extra_params: Vec<(String, QueryValue)>,
    },
    PgOptionsWithTimeouts {
        existing_options: String,
        statement_timeout: Option<ConfiguredSeconds>,
        lock_timeout: Option<ConfiguredSeconds>,
    },
    UrlQueryValue { url: Option<String>, key: String },
    WithQueryValue { url: String, key: String, value: String },
    AppendQueryParams { url: Option<String>, params: Vec<(String, AppendQueryParam)> },
    TokenAuthFlagEnabled { value: Option<FlagValue>, env_var: String },
    StrToBool { value: Option<String> },
}
