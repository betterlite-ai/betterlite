use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::JsonValue;
use betterlite_mcp::oauth_state::query::append_query_params;
use betterlite_mcp::oauth_state::redirect_uris::client_supplied_redirect_uris;
use betterlite_mcp::oauth_state::token_rules::validate_token_response;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

// Internally tagged enums hand each field a ContentDeserializer that cannot serve JsonValue's Box<RawValue>.
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ClientSuppliedRedirectUris {
        value: Value,
    },
    ValidateTokenResponse {
        token_response: Value,
        rules: Value,
        server_id: String,
    },
    AppendQueryParams {
        url: String,
        params: BTreeMap<String, String>,
    },
}

pub(crate) struct Op;

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn json_value(value: &Value) -> Result<JsonValue, Box<HelperError>> {
    let text = serde_json::to_string(value)
        .map_err(|source| helper_error("SerializationError", &source.to_string()))?;
    serde_json::from_str(&text).map_err(|source| helper_error("InvalidInput", &source.to_string()))
}

fn output(value: JsonValue) -> Result<Value, Box<HelperError>> {
    serde_json::to_value(value)
        .map_err(|source| helper_error("SerializationError", &source.to_string()))
}

fn redirect_uris(value: &Value) -> Result<Value, Box<HelperError>> {
    let parsed = json_value(value)?;
    let uris = client_supplied_redirect_uris(Some(&parsed));
    output(uris.map_or(JsonValue::Null, |uris| {
        JsonValue::Array(uris.into_iter().map(JsonValue::String).collect())
    }))
}

fn token_rules(
    token_response: &Value,
    rules: &Value,
    server_id: &str,
) -> Result<Value, Box<HelperError>> {
    let token = json_value(token_response)?;
    let JsonValue::Object(rule_map) = json_value(rules)? else {
        return Err(helper_error("InvalidInput", "rules must be an object"));
    };
    match validate_token_response(&token, &rule_map, server_id) {
        Ok(()) => Ok(Value::Null),
        Err(failure) => Ok(json!({
            "status": 403_u16,
            "detail": {
                "error": "token_validation_failed",
                "server_id": failure.server_id,
                "field": failure.field,
                "message": failure.message,
            },
        })),
    }
}

fn query_params(url: &str, params: &BTreeMap<String, String>) -> Value {
    let pairs: Vec<(String, String)> = params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Value::String(append_query_params(url, &pairs))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ClientSuppliedRedirectUris { value } => redirect_uris(value),
            Input::ValidateTokenResponse {
                token_response,
                rules,
                server_id,
            } => token_rules(token_response, rules, server_id),
            Input::AppendQueryParams { url, params } => Ok(query_params(url, params)),
        }
    }
}
