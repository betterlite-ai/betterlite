use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_bedrock as bedrock;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::json;

#[path = "../../../../../crates/betterlite-bedrock/src/converse_call/tests/fixtures/mod.rs"]
mod fixture;
mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = fixture::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output: fixture::Observation = fixture::run(input).map_err(failure)?;
        let headers: BTreeMap<String, String> = output
            .headers
            .iter()
            .filter(|(name, _)| {
                [
                    "authorization",
                    "content-type",
                    "anthropic-beta",
                    "x-custom",
                ]
                .contains(&name.to_ascii_lowercase().as_str())
            })
            .map(|(name, value)| {
                (
                    name.to_ascii_lowercase(),
                    if value.starts_with("AWS4-HMAC-SHA256") {
                        "sigv4".to_owned()
                    } else {
                        value.clone()
                    },
                )
            })
            .collect();
        let body: Json = serde_json::from_str(&output.body).map_err(failure)?;
        Ok(
            json!({"url": output.url, "body": body, "headers": headers, "status": output.status, "content": output.content, "stream": output.stream, "chunks": output.chunks}),
        )
    }
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ConverseFixture".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
