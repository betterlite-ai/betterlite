use betterlite_db::database_url_policy::flag::{
    FlagValue, TokenFlagError, str_to_bool, token_auth_flag_enabled,
};
use betterlite_db::database_url_policy::policy::{
    ConfiguredSeconds, DatabaseTimeoutSettings, PoolTimeouts, TimeoutError,
    build_db_connection_url_params, pg_options_with_timeouts, unsupported_db_scheme,
    unsupported_db_scheme_message,
};
use betterlite_db::database_url_policy::query::{
    QueryValue, add_missing_query_params, append_query_params, connection_params_from_url,
    idle_lifetime_params, reader_shareable_params, token_refresh_params_from_url, url_query_value,
    with_query_value,
};
use betterlite_db::database_url_policy::url_codec::UrlSplitError;

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
            Input::ConnectionParamsFromUrl { url } => connection_params_from_url(url)
                .map(|params| params_value(&params))
                .map_err(url_split_error),
            Input::TokenRefreshParamsFromUrl { url } => token_refresh_params_from_url(url)
                .map(|params| params_value(&params))
                .map_err(url_split_error),
            Input::UnsupportedDbScheme { database_url } => unsupported_db_scheme(database_url)
                .map(|scheme| scheme.map_or(serde_json::Value::Null, serde_json::Value::String))
                .map_err(url_split_error),
            Input::UnsupportedDbSchemeMessage { env_var, scheme } => {
                Ok(run_unsupported_scheme_message(env_var, scheme))
            }
            Input::BuildDbConnectionUrlParams { .. } => Ok(run_build_connection_params(input)),
            Input::PgOptionsWithTimeouts { .. } => run_pg_options_with_timeouts(input),
            Input::UrlQueryValue { url, key } => run_url_query_value(url.as_deref(), key),
            Input::WithQueryValue { url, key, value } => run_with_query_value(url, key, value),
            Input::AppendQueryParams { url, params } => {
                run_append_query_params(url.as_deref(), params)
            }
            Input::TokenAuthFlagEnabled { value, env_var } => {
                run_token_auth_flag_enabled(value.as_ref(), env_var)
            }
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
    add_missing_query_params(url, params)
        .map(serde_json::Value::String)
        .map_err(url_split_error)
}

fn run_unsupported_scheme_message(env_var: &str, scheme: &str) -> serde_json::Value {
    let message = unsupported_db_scheme_message(env_var, scheme);
    serde_json::Value::String(message)
}

fn run_url_query_value(url: Option<&str>, key: &str) -> crate::helpers::HelperResult {
    url_query_value(url, key)
        .map(serde_json::Value::String)
        .map_err(url_split_error)
}

fn run_with_query_value(url: &str, key: &str, value: &str) -> crate::helpers::HelperResult {
    with_query_value(url, key, value)
        .map(serde_json::Value::String)
        .map_err(url_split_error)
}

fn run_append_query_params(
    url: Option<&str>,
    params: &[(String, Vec<String>)],
) -> crate::helpers::HelperResult {
    append_query_params(url, params)
        .map(|outcome| serde_json::json!({ "url": outcome.url, "warning": outcome.warning }))
        .map_err(url_split_error)
}

fn url_split_error(error: UrlSplitError) -> Box<crate::helpers::HelperError> {
    helper_error("ValueError", error.to_string())
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
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

fn run_token_auth_flag_enabled(
    value: Option<&FlagValue>,
    env_var: &str,
) -> crate::helpers::HelperResult {
    let result = token_auth_flag_enabled(value, env_var);
    result.map(serde_json::Value::Bool).map_err(|error| {
        let TokenFlagError::NotRecognized { message } = error;
        Box::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        })
    })
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    IdleLifetimeParams {
        configured: Option<QueryValue>,
    },
    AddMissingQueryParams {
        url: String,
        params: Vec<(String, QueryValue)>,
    },
    ReaderShareableParams {
        params: Vec<(String, QueryValue)>,
    },
    ConnectionParamsFromUrl {
        url: String,
    },
    TokenRefreshParamsFromUrl {
        url: String,
    },
    UnsupportedDbScheme {
        database_url: String,
    },
    UnsupportedDbSchemeMessage {
        env_var: String,
        scheme: String,
    },
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
    UrlQueryValue {
        url: Option<String>,
        key: String,
    },
    WithQueryValue {
        url: String,
        key: String,
        value: String,
    },
    AppendQueryParams {
        url: Option<String>,
        params: Vec<(String, Vec<String>)>,
    },
    TokenAuthFlagEnabled {
        value: Option<FlagValue>,
        env_var: String,
    },
    StrToBool {
        value: Option<String>,
    },
}
