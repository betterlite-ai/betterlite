use betterlite_db::database_tls::translation::{
    libpq_tls_params, strip_prisma_query_params, translate_libpq_ssl_params,
};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    TranslateLibpqSslParams {
        url: String,
    },
    StripPrismaQueryParams {
        url: String,
    },
    LibpqTlsParams {
        pairs: Vec<(String, String)>,
        sslaccept: Option<String>,
    },
    Batch {
        calls: Vec<Self>,
    },
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::TranslateLibpqSslParams { url } => translate_value(url),
            Input::StripPrismaQueryParams { url } => strip_value(url),
            Input::LibpqTlsParams { pairs, sslaccept } => {
                Ok(pairs_value(&libpq_tls_params(pairs, sslaccept.as_deref())))
            }
            Input::Batch { calls } => Ok(serde_json::Value::Array(
                calls.iter().map(scenario_value).collect(),
            )),
        }
    }
}

fn translate_value(url: &str) -> HelperResult {
    let mut resolved: Vec<(String, String, u16)> = Vec::new();
    let translated = translate_libpq_ssl_params(url, |path, host, port| {
        resolved.push((path.to_owned(), host.to_owned(), port));
        path.to_owned()
    })
    .map_err(|error| invalid_value(&error.to_string()))?;
    let calls: Vec<serde_json::Value> = resolved
        .iter()
        .map(|(path, host, port)| serde_json::json!([path, host, port]))
        .collect();
    Ok(serde_json::json!({"url": translated, "resolved": calls}))
}

fn strip_value(url: &str) -> HelperResult {
    let stripped =
        strip_prisma_query_params(url).map_err(|error| invalid_value(&error.to_string()))?;
    Ok(serde_json::Value::String(stripped))
}

fn scenario_value(input: &Input) -> serde_json::Value {
    match Op::run(input) {
        Ok(value) => serde_json::json!({"ok": true, "value": value}),
        Err(_error) => serde_json::json!({"ok": false}),
    }
}

fn pairs_value(pairs: &[(String, String)]) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = pairs
        .iter()
        .map(|(key, value)| serde_json::json!([key, value]))
        .collect();
    serde_json::Value::Array(entries)
}

fn invalid_value(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
