use std::collections::BTreeMap;

use betterlite_core::api::responses_compact_call::call::CompactResponsesCall;
use betterlite_core::api::responses_compact_call::deps::{
    CompactConfigFactory, CompactResponsesDeps,
};
use betterlite_core::api::responses_compact_http::service::{ClientSource, CompactTransport};
use betterlite_models::call_helpers::drop_params::drop_params_env_flag;
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::{ReplyBody, ResponsesReply};
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::projection::failure;
use super::support::{Configs, Fixture, Outbound, Registry, ScriptedResolver, scripted};
use super::{Call, Outcome, Run, RunOutcome};

// The compact wrapper over the case env's key and the mock base, with the scripted provider registry
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Outcome, SdkHttpError> {
    let origin = Outbound::new(env)?;
    let resolver = ScriptedResolver;
    let lookup = Registry;
    let configs = Configs {
        api_key: env.get("OPENAI_API_KEY").cloned().map(SecretString::from),
    };
    let drop_params =
        drop_params_env_flag(env.get("LITELLM_DROP_PARAMS").map(String::as_str)).enabled;
    let deps = CompactResponsesDeps {
        resolver: &resolver,
        lookup: &lookup,
        configs: &configs,
        source: &origin,
        environment: env,
        drop_params,
    };
    let mut fixture = Fixture::new("responses")?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, api_base, &deps, &mut fixture).await?);
    }
    Ok(Outcome { runs })
}

async fn run_once<S, F>(
    run: &Run,
    api_base: &str,
    deps: &CompactResponsesDeps<'_, S, F>,
    fixture: &mut Fixture,
) -> Result<RunOutcome, SdkHttpError>
where
    S: ClientSource<Client: CompactTransport>,
    F: CompactConfigFactory,
{
    let request = compact_call(run, api_base);
    match fixture.run(deps, &request, run.mode).await {
        Ok(reply) => Ok(RunOutcome {
            result: Some(reply_json(reply)?),
            failure: None,
        }),
        Err(error) => {
            let (provider, model) = scripted(&run.model, run.custom_llm_provider.as_deref());
            Ok(RunOutcome {
                result: None,
                failure: Some(failure(&error, &provider, &model, &fixture.redactor)),
            })
        }
    }
}

fn compact_call(run: &Run, api_base: &str) -> CompactResponsesCall {
    CompactResponsesCall {
        model: run.model.clone(),
        input: run.input.clone(),
        instructions: run.instructions.clone(),
        previous_response_id: run.previous_response_id.clone(),
        custom_llm_provider: run.custom_llm_provider.clone(),
        api_key: None,
        api_base: Some(api_base.to_owned()),
        ssl_verify: None,
        extra_headers: run.extra_headers.clone(),
        extra_query: run.extra_query.clone(),
        extra_body: run.extra_body.clone(),
        timeout: run.timeout_seconds,
        shared_session: None,
        kwargs: run.kwargs.clone(),
        acompact_responses: None,
    }
}

// The reply as LiteLLM's ResponsesAPIResponse dumps it: a validated reply as the typed model, a lax one as sent
fn reply_json(reply: ResponsesReply) -> Result<Json, SdkHttpError> {
    match reply.response {
        ReplyBody::Typed(response) => {
            let value = serde_json::to_value(*response).map_err(SdkHttpError::Serialize)?;
            serde_json::from_value(value).map_err(|error| SdkHttpError::Invalid(error.to_string()))
        }
        ReplyBody::Lax(fields) => Ok(Json::Object(fields)),
    }
}
