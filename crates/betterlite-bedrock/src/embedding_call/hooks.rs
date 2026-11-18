use std::vec::IntoIter;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions;
use betterlite_openai::cohere_legacy_embedding::PreCall as CoherePreCall;
use betterlite_openai::cohere_legacy_embedding::{EmbeddingHooks, PostCall as CoherePostCall};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;

use super::LogContext;
use super::error::EmbeddingCallError;

pub(super) enum Input {
    Texts(Vec<String>),
    Body,
}

pub(super) enum Event {
    Pre {
        input: Input,
        body: Object,
        headers: Option<Headers>,
        api_base: Option<String>,
    },
    Post {
        input: Input,
        body: Object,
        original_response: Json,
    },
}

#[derive(Default)]
pub(super) struct Recorder {
    events: Vec<Event>,
}

impl EmbeddingHooks for Recorder {
    // the cohere branch logs its pre call before the send, so only the post call waits for the reply
    fn pre_call(&mut self, _call: CoherePreCall<'_>) {}

    fn post_call(&mut self, call: CoherePostCall<'_>) {
        self.events.push(Event::Post {
            input: Input::Texts(call.input.to_vec()),
            body: call.body.clone(),
            original_response: call.original_response.clone(),
        });
    }
}

impl IntoIterator for Recorder {
    type Item = Event;
    type IntoIter = IntoIter<Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

// the hooks borrow the logging state, which is not Send, so B1418 records its calls and B249 logs them here
pub(super) fn replay<Events>(
    events: Events,
    log: &mut LogContext<'_, '_>,
) -> Result<(), EmbeddingCallError>
where
    Events: IntoIterator<Item = Event>,
{
    for event in events {
        log_event(event, log)?;
    }
    Ok(())
}

pub(super) fn log_event(
    event: Event,
    log: &mut LogContext<'_, '_>,
) -> Result<(), EmbeddingCallError> {
    match event {
        Event::Pre {
            input,
            body,
            headers,
            api_base,
        } => {
            let logged = headers.as_ref().map(logged_headers);
            pre_call(
                log.state,
                &PreCall {
                    input: input_value(input, &body)?,
                    model: None,
                    additional_args: arguments(api_base, logged, &body)?,
                },
                log.env,
            )?;
        }
        Event::Post {
            input,
            body,
            original_response,
        } => {
            post_call(
                log.state,
                &PostCall {
                    original_response: serde_json::from_value(serde_json::to_value(
                        &original_response,
                    )?)?,
                    input: input_value(input, &body)?,
                    additional_args: arguments(None, None, &body)?,
                },
                log.env,
            );
        }
    }
    Ok(())
}

fn input_value(input: Input, body: &Object) -> Result<decisions::Json, EmbeddingCallError> {
    match input {
        Input::Texts(texts) => Ok(decisions::Json::List(
            texts.into_iter().map(decisions::Json::Text).collect(),
        )),
        Input::Body => Ok(decisions::Json::Object(serde_json::from_value(
            serde_json::to_value(body)?,
        )?)),
    }
}

fn arguments(
    api_base: Option<String>,
    headers: Option<IndexMap<String, decisions::Json>>,
    body: &Object,
) -> Result<AdditionalArgs, EmbeddingCallError> {
    Ok(AdditionalArgs {
        api_base,
        headers,
        complete_input_dict: Some(RawBody::Object(serde_json::from_value(
            serde_json::to_value(body)?,
        )?)),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    })
}

fn logged_headers(headers: &Headers) -> IndexMap<String, decisions::Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), decisions::Json::Text(value.clone())))
        .collect()
}
