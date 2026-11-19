use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::responses_read_http::config::{
    NoWorkloadIdentity, OpenAiReadConfig, ReadEnvironment, ReadParams,
};
use betterlite_core::api::responses_read_http::error::ReadError;
use betterlite_core::api::responses_read_http::service::{
    InputItemsCall, ReadCall, ReadDeps, ReadOutcome, ReadTarget, RetrieveCall, send,
};
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use betterlite_openai::response_resources::request::ListInputItemsQuery;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, Source, source};
use super::{Call, Failure, Operation, Outcome, Retrieved, Run, RunOutcome};

// The production read service over the mock's base: the same config sections and pooled client a request would build
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let settings = &config.sections.resource_sdk_settings;
    let provider = OpenAiReadConfig::new(
        LiteLlmGlobals::default(),
        ReadEnvironment {
            api_key: settings.openai_api_key.clone(),
            base_url: settings.openai_base_url.clone(),
            api_base: settings.openai_api_base.clone(),
        },
        NoWorkloadIdentity,
    );
    let origin = source(&config, env)?;
    let params = ReadParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    };
    let mut fixture = Fixture::new()?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        runs.push(run_once(run, &provider, &origin, &params, &mut fixture).await?);
    }
    Ok(SdkResult::ResponsesReadTransport {
        value: Box::new(Outcome { runs }),
    })
}

async fn run_once(
    run: &Run,
    config: &OpenAiReadConfig<NoWorkloadIdentity>,
    origin: &Source,
    params: &ReadParams,
    fixture: &mut Fixture,
) -> Result<RunOutcome, SdkHttpError> {
    let query = run.query.list_input_items();
    let call = read_call(run, params, &query);
    let deps = ReadDeps {
        config,
        source: origin,
        supplied: None,
    };
    let prepared = fixture.prepare(&deps, run.mode.client_kind(), &call)?;
    outcome(run.operation, send(prepared).await)
}

fn read_call<'call>(
    run: &'call Run,
    params: &'call ReadParams,
    query: &'call ListInputItemsQuery,
) -> ReadCall<'call> {
    let target = ReadTarget {
        response_id: &run.response_id,
        params,
        extra_headers: run.extra_headers.as_ref(),
        timeout: run.timeout_seconds.map(Duration::from_secs),
    };
    match run.operation {
        Operation::Retrieve => ReadCall::Retrieve(RetrieveCall {
            target,
            extra_body: run.extra_body.as_ref(),
        }),
        Operation::InputItems => ReadCall::InputItems(InputItemsCall { target, query }),
    }
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(
    operation: Operation,
    result: Result<ReadOutcome, ReadError>,
) -> Result<RunOutcome, SdkHttpError> {
    match result {
        Ok(ReadOutcome::Response(reply)) if operation == Operation::Retrieve => Ok(RunOutcome {
            retrieved: Some(Retrieved {
                id: reply.response.id.clone(),
                created_at: reply.response.created_at,
                status: reply.response.status.clone(),
                model: reply.response.model.clone(),
            }),
            result: None,
            failure: None,
            validation: false,
        }),
        Ok(ReadOutcome::Response(_)) => Err(SdkHttpError::Invalid(
            "the input items read returned a typed response".to_owned(),
        )),
        Ok(ReadOutcome::InputItems(value)) => Ok(RunOutcome {
            retrieved: None,
            result: Some(value),
            failure: None,
            validation: false,
        }),
        Err(ReadError::Provider(error)) => Ok(RunOutcome {
            retrieved: None,
            result: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: Some(error.headers),
            }),
            validation: false,
        }),
        Err(ReadError::Response(ResponseResourceError::Provider(OpenAiError::Provider(
            provider,
        )))) => Ok(RunOutcome {
            retrieved: None,
            result: None,
            failure: Some(Failure {
                status: provider.status_code,
                synthesized: provider.status_code_is_synthesized,
                message: provider.message,
                headers: failure_headers(provider.headers),
            }),
            validation: false,
        }),
        Err(ReadError::Response(ResponseResourceError::Record { .. })) => Ok(RunOutcome {
            retrieved: None,
            result: None,
            failure: None,
            validation: true,
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn failure_headers(headers: Option<Vec<(String, String)>>) -> Option<BTreeMap<String, Json>> {
    headers.map(|pairs| {
        pairs
            .into_iter()
            .map(|(name, value)| (name, Json::Text(value)))
            .collect()
    })
}
