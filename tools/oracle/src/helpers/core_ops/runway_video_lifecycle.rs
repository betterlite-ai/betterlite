use std::fmt::Display;

use betterlite_core::api::runway::video_lifecycle::cancel::{cancel_request, cancel_response};
use betterlite_core::api::runway::video_lifecycle::content::content_request;
use betterlite_core::api::runway::video_lifecycle::error::VideoLifecycleError;
use betterlite_core::api::runway::video_lifecycle::retrieve::{
    retrieve_request, retrieve_response,
};
use betterlite_core::api::runway::video_lifecycle::unsupported::{
    UnsupportedOperation, unsupported_request,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

mod content;

use self::content::content;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    kind: RequestKind,
    video_id: String,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RequestKind {
    Content,
    Status,
    Delete,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseCall {
    kind: ResponseKind,
    #[serde(default)]
    provider: Option<String>,
    body: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ResponseKind {
    Retrieve,
    Delete,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentCall {
    body: String,
    reply: ReplyCall,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    status: u16,
    body: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UnsupportedCall {
    Remix,
    List,
    CreateCharacter,
    GetCharacter,
    Edit,
    Extension,
}

impl UnsupportedCall {
    const fn operation(self) -> UnsupportedOperation {
        match self {
            Self::Remix => UnsupportedOperation::Remix,
            Self::List => UnsupportedOperation::List,
            Self::CreateCharacter => UnsupportedOperation::CreateCharacter,
            Self::GetCharacter => UnsupportedOperation::GetCharacter,
            Self::Edit => UnsupportedOperation::Edit,
            Self::Extension => UnsupportedOperation::Extension,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Request(Vec<RequestCall>),
    Response(Vec<ResponseCall>),
    Content(Vec<ContentCall>),
    Unsupported(Vec<UnsupportedCall>),
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RunwayVideoLifecycleError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn request(calls: &[RequestCall]) -> HelperResult {
    let mut urls = Vec::with_capacity(calls.len());
    for call in calls {
        let api_base = call.api_base.as_deref();
        let request = match call.kind {
            RequestKind::Content => {
                content_request(&call.video_id, api_base, call.variant.as_deref())
            }
            RequestKind::Status => retrieve_request(&call.video_id, api_base),
            RequestKind::Delete => cancel_request(&call.video_id, api_base),
        }
        .map_err(failure)?;
        urls.push(json!({"url": request.url, "data": request.data}));
    }
    Ok(json!({"urls": urls}))
}

fn response(calls: &[ResponseCall]) -> HelperResult {
    let mut videos = Vec::with_capacity(calls.len());
    for call in calls {
        let video = match call.kind {
            ResponseKind::Retrieve => {
                retrieve_response(call.body.as_bytes(), call.provider.as_deref())
            }
            ResponseKind::Delete => cancel_response(call.body.as_bytes()),
        }
        .map_err(failure)?;
        videos.push(json!(video));
    }
    Ok(json!({"videos": videos}))
}

fn unsupported(calls: &[UnsupportedCall]) -> HelperResult {
    let mut messages = Vec::with_capacity(calls.len());
    for call in calls {
        match unsupported_request(call.operation()) {
            Err(VideoLifecycleError::Unsupported { message }) => messages.push(json!(message)),
            Err(error) => return Err(failure(error)),
            Ok(never) => match never {},
        }
    }
    Ok(json!({"messages": messages}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Request(calls) => request(calls),
            Input::Response(calls) => response(calls),
            Input::Content(calls) => content(calls),
            Input::Unsupported(calls) => unsupported(calls),
        }
    }
}
