use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_history::convert::convert_messages_with_history;
use betterlite_vertex::gemini_history::error::HistoryError;
use serde::Deserialize;
use serde_json::{Value, json};

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
    data_urls: BTreeMap<String, String>,
}

struct MapSource<'urls> {
    data_urls: &'urls BTreeMap<String, String>,
}

impl MapSource<'_> {
    fn result_for(&self, url: &str) -> Result<String, ImageFetchError> {
        self.data_urls
            .get(url)
            .cloned()
            .ok_or_else(|| ImageFetchError::Attempts {
                url: url.to_owned(),
            })
    }
}

impl RemoteMediaFetcher for MapSource<'_> {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(self.result_for(url))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        ready(urls.iter().map(|url| self.result_for(url)).collect())
    }
}

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| helper_error("RuntimeError", error))?;
    let mut results = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        let source = MapSource {
            data_urls: &case.data_urls,
        };
        let outcome = runtime.block_on(convert_messages_with_history(
            &case.messages,
            case.model.as_deref(),
            &source,
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
            Err(error) => error_value(&error),
        });
    }
    Ok(json!({"results": results}))
}

fn error_value(error: &HistoryError) -> Value {
    let wire = error.wire();
    json!({"error": {
        "status": wire.status,
        "type": wire.error_type,
        "param": wire.param,
        "code": wire.code,
    }})
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
