use betterlite_bedrock::sagemaker_runtime::chat::{ChatCall, ChatVariant, prepare_chat};
use betterlite_bedrock::sagemaker_runtime::endpoint::ParamMap;
use betterlite_bedrock::sagemaker_runtime::error::RuntimeFailure;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_wire::{NEW_ID, ReplyOptions, system_clock};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::enterprise_ml_dispatch::call::convert;
use http::{HeaderMap, HeaderName, HeaderValue};

use super::super::case::{Case, Operation};
use super::super::fixture::Fixture;
use super::super::wire::{Failure, Sent, failure};

fn headers_from(pairs: &[(String, String)]) -> Result<HeaderMap, Failure> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let lowered = name.to_ascii_lowercase();
        let header_name =
            HeaderName::from_bytes(lowered.as_bytes()).map_err(|error| failure(None, &error))?;
        let header_value = HeaderValue::from_str(value).map_err(|error| failure(None, &error))?;
        headers.insert(header_name, header_value);
    }
    Ok(headers)
}

// _complete_sagemaker_chat: the context provider reaches B5960's own chat config, so its body and signature are built here
pub(in crate::b8331) async fn sagemaker_chat(
    case: &Case,
    fixture: &Fixture,
) -> Result<(), Failure> {
    let messages: Vec<ChatMessage> =
        convert(&case.messages).map_err(|error| failure(error.status(), &error))?;
    let optional_params = Object::new();
    let signing = ParamMap::new();
    let runtime = fixture.runtime();
    let variant = if case.op == Operation::SagemakerNova {
        ChatVariant::Nova
    } else {
        ChatVariant::Chat
    };
    let call = ChatCall {
        model: &case.model,
        messages: &messages,
        optional_params: &optional_params,
        signing: &signing,
        headers: Headers::new(),
        hf_model_name: None,
        variant,
        stream: false,
        reply: ReplyOptions {
            json_mode: false,
            provider: None,
            base_model: None,
            identifier: NEW_ID,
            clock: system_clock,
        },
        configured_bearer: None,
    };
    let prepared = prepare_chat(&runtime, &call).await.map_err(|error| {
        let raised = RuntimeFailure::from(error);
        failure(raised.status_code(), &raised)
    })?;
    fixture.transport.keep(Sent {
        method: String::from("POST"),
        url: prepared.url,
        headers: headers_from(&prepared.headers)?,
        body: Some(prepared.body),
    });
    Ok(())
}
