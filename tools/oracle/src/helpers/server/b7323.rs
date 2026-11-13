use betterlite_server::custom_url::{get_custom_url, join_paths};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    JoinPaths {
        base_path: String,
        route: String,
    },
    GetCustomUrl {
        proxy_base_url: Option<String>,
        request_base_url: String,
        request_root_path: String,
        route: Option<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::JoinPaths { base_path, route } => Value::String(join_paths(base_path, route)),
            Input::GetCustomUrl {
                proxy_base_url,
                request_base_url,
                request_root_path,
                route,
            } => Value::String(get_custom_url(
                proxy_base_url.as_deref(),
                request_base_url,
                request_root_path,
                route.as_deref(),
            )),
        };
        Ok(output)
    }
}
