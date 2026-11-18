use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions;
use betterlite_openai::cohere_legacy_embedding::PreCall as CoherePreCall;
use betterlite_openai::cohere_legacy_embedding::{EmbeddingHooks, PostCall as CoherePostCall};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;

use super::error::EmbeddingCallError;
use super::{LogContext, decisions_list, decisions_object, decisions_value};
use crate::signing::request::AwsSignedRequest;

pub enum Event {
    Pre {
        input: Vec<String>,
        body: Object,
        headers: Option<Headers>,
        api_base: Option<String>,
    },
    Post {
        input: Vec<String>,
        body: Object,
        original_response: Json,
    },
}

#[derive(Default)]
pub struct Recorder {
    events: Vec<Event>,
}

impl EmbeddingHooks for Recorder {
    fn pre_call(&mut self, call: CoherePreCall<'_>) {
        self.events.push(Event::Pre {
            input: call.input.to_vec(),
            body: call.body.clone(),
            headers: call.headers.cloned(),
            api_base: call.api_base.map(str::to_owned),
        });
    }

    fn post_call(&mut self, call: CoherePostCall<'_>) {
        self.events.push(Event::Post {
            input: call.input.to_vec(),
            body: call.body.clone(),
            original_response: call.original_response.clone(),
        });
    }
}

impl Recorder {
    pub fn into_events(self) -> Vec<Event> {
        self.events
    }
}

// The hooks borrow the logging state, which is not Send, so B1418's execute records its calls and
// they reach B249 here in the same pre-then-post order.
pub fn replay(events: Vec<Event>, log: &mut LogContext<'_, '_>) -> Result<(), EmbeddingCallError> {
    for event in events {
        match event {
            Event::Pre {
                input,
                body,
                headers,
                api_base,
            } => pre_call(
                log.state,
                &PreCall {
                    input: decisions_list(&input),
                    model: None,
                    additional_args: AdditionalArgs {
                        api_base,
                        headers: headers.as_ref().map(logged_headers),
                        complete_input_dict: Some(RawBody::Object(decisions_object(&body)?)),
                        request_str: None,
                        openai_sdk: None,
                        passthrough: IndexMap::new(),
                    },
                },
                log.env,
            )?,
            Event::Post {
                input,
                body,
                original_response,
            } => post_call(
                log.state,
                &PostCall {
                    original_response: decisions_value(&original_response)?,
                    input: decisions_list(&input),
                    additional_args: AdditionalArgs {
                        api_base: None,
                        headers: None,
                        complete_input_dict: Some(RawBody::Object(decisions_object(&body)?)),
                        request_str: None,
                        openai_sdk: None,
                        passthrough: IndexMap::new(),
                    },
                },
                log.env,
            ),
        }
    }
    Ok(())
}

fn logged_headers(headers: &Headers) -> IndexMap<String, decisions::Json> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), decisions::Json::Text(value.clone())))
        .collect()
}

// the per-input path logs its own request and reply around B693's call
pub fn log_request(
    data: &Object,
    signed: &AwsSignedRequest,
    log: &mut LogContext<'_, '_>,
) -> Result<(), EmbeddingCallError> {
    let body = decisions_object(data)?;
    pre_call(
        log.state,
        &PreCall {
            input: decisions::Json::Object(body.clone()),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(signed.url.clone()),
                headers: Some(logged_headers(&signed.headers.iter().cloned().collect())),
                complete_input_dict: Some(RawBody::Object(body)),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    )?;
    Ok(())
}

pub fn log_reply(
    data: &Object,
    response: &Object,
    log: &mut LogContext<'_, '_>,
) -> Result<(), EmbeddingCallError> {
    let body = decisions_object(data)?;
    post_call(
        log.state,
        &PostCall {
            original_response: decisions::Json::Object(decisions_object(response)?),
            input: decisions::Json::Object(body.clone()),
            additional_args: AdditionalArgs {
                api_base: None,
                headers: None,
                complete_input_dict: Some(RawBody::Object(body)),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        },
        log.env,
    );
    Ok(())
}
