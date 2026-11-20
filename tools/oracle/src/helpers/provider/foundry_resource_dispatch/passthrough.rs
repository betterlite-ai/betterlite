use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{
    AzureRelayedLog, NoChatReplyProjection, NoChatStreamAssembly,
};
use betterlite_foundry::passthrough::logging::FoundryRelayedLog;
use betterlite_foundry::passthrough::plan::{FoundryLogInput, FoundryPassthroughPlan};
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::PassthroughTarget;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::passthrough::{Call, Dispatch, Outcome};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceOperation, FoundryResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Runtime;

use super::support::{OTHER, RelayLog, failure, runtime};

#[derive(Debug, Deserialize)]
pub(super) struct Input {
    model: String,
    endpoint: String,
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    body: String,
    request_url: Option<String>,
}

const fn default_status() -> u16 {
    200
}

// B566's response projection, including the standard body an unknown route keeps
pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    nonstream_answer(input, &runtime)
}

fn plan() -> FoundryPassthroughPlan {
    FoundryPassthroughPlan {
        global_api_base: None,
        global_api_key: None,
        azure: AzurePassthroughConfig {
            api_base_env: None,
            api_key_env: None,
            model_call_details: None,
            chat_reply: NoChatReplyProjection,
            chat_stream: NoChatStreamAssembly,
        },
        ocr: None,
        litellm_params: Object::new(),
    }
}

const fn target<'call>(model: &'call str, endpoint: &'call str) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure_ai",
        endpoint,
    }
}

fn nonstream_answer(input: &Input, runtime: &Runtime) -> HelperResult {
    let plan = plan();
    let headers = Headers::new();
    let reply = ResponseReply {
        status: input.status,
        headers: &headers,
        body: input.body.as_bytes(),
    };
    let target = target(&input.model, &input.endpoint);
    let request_url = input.request_url.as_deref().unwrap_or(&input.endpoint);
    let request_data = Object::new();
    let relayed = FoundryLogInput {
        target: &target,
        reply: &reply,
        request_data: &request_data,
        request_url,
    };
    let mut logging = RelayLog::default();
    let operation = FoundryResourceOperation::Passthrough(Box::new(Dispatch {
        plan: &plan,
        logging: &mut logging,
        call: Call::Relayed(relayed),
    }));
    let outcome = match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::Passthrough(Outcome::Relayed(logged))) => logged,
        Ok(_) => return Err(failure("Variant", OTHER)),
        Err(error) => return Err(failure("LeafFailure", error)),
    };
    let (kind, standard) = outcome_fields(outcome.as_deref());
    Ok(json!({"kind": kind, "call_type": logging.call_type, "standard": standard}))
}

fn outcome_fields(outcome: Option<&FoundryRelayedLog>) -> (&'static str, Value) {
    match outcome {
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Chat(_))) => ("chat", Value::Null),
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Relay(_)) | FoundryRelayedLog::Relay(_)) => {
            ("relay", Value::Null)
        }
        Some(FoundryRelayedLog::Azure(AzureRelayedLog::Terminal(_))) => ("terminal", Value::Null),
        Some(FoundryRelayedLog::Ocr(_)) => ("ocr", Value::Null),
        Some(FoundryRelayedLog::Standard(response)) => (
            "standard",
            serde_json::to_value(&response.response).unwrap_or(Value::Null),
        ),
        None => ("none", Value::Null),
    }
}
