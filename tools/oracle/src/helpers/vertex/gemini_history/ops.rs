use std::future::{Future, ready};

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_history::convert::convert_messages_with_history;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: Vec<Case>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Case {
    messages: Vec<Message>,
    model: Option<String>,
}

struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| runtime_error(&error))?;
    let mut results = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        let outcome = runtime.block_on(convert_messages_with_history(
            &case.messages,
            case.model.as_deref(),
            &NoFetch,
        ));
        results.push(match outcome {
            Ok(history) => json!({
                "contents": history.contents,
                "warnings": history
                    .warnings
                    .iter()
                    .map(|warning| warning.subject())
                    .collect::<Vec<&str>>(),
            }),
            Err(error) => json!({"error": {
                "status": error.status(),
                "type": error.error_type(),
                "param": error.param(),
                "code": error.code(),
            }}),
        });
    }
    Ok(json!({"results": results}))
}

fn runtime_error(error: &std::io::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RuntimeError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
