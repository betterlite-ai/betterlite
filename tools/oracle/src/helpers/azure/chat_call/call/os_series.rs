use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::chat_call::client::ClientHandle;
use betterlite_azure::chat_call::o_series::{
    self, OSeriesCall, OSeriesDelegate, OSeriesError, OSeriesForward,
};
use betterlite_azure::endpoints::projection::ClientKind;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_prompts::arguments::json::Object;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::super::CallCase;
use super::super::hooks::Recorder;
use super::super::source::{ScriptedSource, ScriptedTransport};
use super::super::view::{self, Outcome};
use super::{Secrets, block_on, litellm_params, message_text, optional_object, source_of};

struct Delegate {
    forward: Arc<Mutex<Value>>,
}

fn kind_name(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::AzureOpenAi => "azure_sync",
        ClientKind::AsyncAzureOpenAi => "azure_async",
        ClientKind::OpenAi => "openai_sync",
        ClientKind::AsyncOpenAi => "openai_async",
        ClientKind::Other => "other",
    }
}

fn forward_value(call: &OSeriesForward<'_, ScriptedTransport>) -> Value {
    let params = call.litellm_params;
    json!({
        "model": call.model,
        "api_base": call.api_base,
        "api_version": call.api_version,
        "api_key": message_text(call.api_key),
        "azure_ad_token": message_text(call.azure_ad_token),
        "timeout": call.timeout,
        "acompletion": call.acompletion,
        "organization": call.organization,
        "drop_params": call.drop_params,
        "shared_session": call.shared_session,
        "dynamic_params": call.dynamic_params,
        "headers": call.headers.as_ref().map_or(Value::Null, |headers| {
            Value::Object(
                headers
                    .0
                    .iter()
                    .map(|(name, value)| (name.clone(), Value::String(value.clone())))
                    .collect(),
            )
        }),
        "optional_params": serde_json::to_value(call.optional_params).unwrap_or(Value::Null),
        "litellm_params": {
            "azure_ad_token": params.azure_ad_token.as_ref().map(|token| token.expose_secret().to_owned()),
            "max_retries": params.max_retries,
            "timeout": params.timeout,
        },
        "client": {
            "kind": kind_name(call.client.kind),
            "api_key": message_text(call.client.api_key.as_ref()),
        },
    })
}

impl OSeriesDelegate for Delegate {
    type Client = ScriptedTransport;
    type Reply = Value;
    type Error = String;

    fn completion<'call>(
        &'call self,
        call: OSeriesForward<'call, Self::Client>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Reply, Self::Error>> + Send + 'call>> {
        let forward = forward_value(&call);
        *self.forward.lock().unwrap_or_else(PoisonError::into_inner) = forward.clone();
        Box::pin(async move { Ok(forward) })
    }
}

fn o_series_call<'call>(
    case: &'call CallCase,
    secrets: &'call Secrets,
    params: &'call Object,
    supplied: Option<ClientHandle<ScriptedTransport>>,
) -> OSeriesCall<'call, ScriptedTransport> {
    OSeriesCall {
        model: case.model.as_deref(),
        messages: case.messages.as_deref(),
        api_key: secrets.key.as_ref(),
        api_base: Some(&case.api_base),
        api_version: Some(&case.api_version),
        azure_ad_token: secrets.token.as_ref(),
        timeout: Some(case.timeout),
        optional_params: params,
        litellm_params: litellm_params(case),
        acompletion: case.acompletion,
        headers: secrets.headers.as_ref(),
        organization: case.organization.as_deref(),
        drop_params: case.drop_params,
        shared_session: None,
        dynamic_params: None,
        supplied,
    }
}

fn report(
    source: &ScriptedSource,
    recorder: &Recorder,
    outcome: Result<Value, OSeriesError<String>>,
) -> Value {
    let outcome = match outcome {
        Ok(forward) => Outcome {
            name: "delegated",
            reply: Value::Null,
            error: Value::Null,
            forward,
        },
        Err(OSeriesError::Client(error)) => Outcome {
            name: "error",
            reply: Value::Null,
            error: json!({"status": error.status()}),
            forward: Value::Null,
        },
        Err(OSeriesError::Delegate(_message)) => Outcome {
            name: "error",
            reply: Value::Null,
            error: Value::Null,
            forward: Value::Null,
        },
    };
    view::report(source, recorder, &outcome)
}

pub(crate) fn run(case: &CallCase) -> Result<Value, Box<HelperError>> {
    let source = source_of(case);
    let secrets = Secrets::of(case);
    let params = optional_object(case)?;
    let supplied = case.supplied.then(|| source.handle());
    let call = o_series_call(case, &secrets, &params, supplied);
    let delegate = Delegate {
        forward: Arc::new(Mutex::new(Value::Null)),
    };
    let recorder = Recorder::default();
    let outcome = block_on(o_series::completion(&source, &call, &delegate));
    Ok(report(&source, &recorder, outcome))
}
