use std::cell::Cell;
use std::future::ready;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::batch_create::reply::{
    OriginalBatchRequest, create_batch_reply, openai_batch_metadata,
};
use betterlite_bedrock::batch_create::request::build::{CreateBatchInput, build_create_request};
use betterlite_bedrock::batch_create::request::sign::transform_create_batch_request;
use betterlite_bedrock::batch_create::titan::titan_batch_token_usage;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use serde_json::{Value, json};

use super::input::{Input, Operation, Scenario};
use super::support::{epoch, runtime, secrets};
use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

pub(super) fn run(input: &Input) -> HelperResult {
    let call: fn(&Scenario) -> Value = match input.operation {
        Operation::MetadataSanitization => {
            return values_operation(input, |decoded| {
                serde_json::to_value(openai_batch_metadata(decoded)).or_value_error()
            });
        }
        Operation::TitanBatchTokenUsage => {
            return values_operation(input, |decoded| {
                serde_json::to_value(titan_batch_token_usage(decoded)).or_value_error()
            });
        }
        Operation::CreateSignedRequest => signed_call,
        Operation::CreateStatusMapping | Operation::CreateReplyDefaults => reply_call,
        Operation::BatchRoleRequired
        | Operation::OutputBucketFallback
        | Operation::TagsValidation
        | Operation::Input24hWindow
        | Operation::GeneratedJobName
        | Operation::OwnerAndKms => build_call,
    };
    Ok(json!({"calls": input.scenarios.iter().map(call).collect::<Vec<Value>>()}))
}

fn build_call(scenario: &Scenario) -> Value {
    match build_scenario(scenario) {
        Ok(request) => json!({"failed": false, "request": request}),
        Err(_error) => json!({"failed": true}),
    }
}

fn build_scenario(scenario: &Scenario) -> Result<Value, Box<HelperError>> {
    let secrets = secrets();
    let input = create_input(scenario, &secrets)?;
    let request = build_create_request(&input).or_value_error()?;
    serde_json::to_value(&request).or_value_error()
}

fn create_input<'text>(
    scenario: &'text Scenario,
    secrets: &'text SecretLookup,
) -> Result<CreateBatchInput<'text>, Box<HelperError>> {
    Ok(CreateBatchInput {
        model: &scenario.model,
        input_file_id: scenario.input_file_id.as_deref(),
        completion_window: scenario.completion_window.as_deref(),
        deployment_params: &scenario.litellm_params,
        request_params: &scenario.optional_params,
        environment: &scenario.environment,
        secrets,
        job_prefix: scenario.job_prefix.as_deref(),
        unique_id: &scenario.uuid,
        region_inputs: RegionInputs {
            environment_region_name: scenario
                .environment
                .get("AWS_REGION_NAME")
                .map(String::as_str),
            environment_region: scenario.environment.get("AWS_REGION").map(String::as_str),
            session_region: None,
        },
        now: epoch(scenario.now)?,
    })
}

fn signed_call(scenario: &Scenario) -> Value {
    let Some(credentials) = scenario.credentials.as_ref() else {
        return json!({"failed": true});
    };
    let held = Credentials {
        access_key_id: credentials.access_key_id.clone(),
        secret_access_key: credentials.secret_access_key.clone(),
        session_token: credentials.session_token.clone(),
    };
    let secrets = secrets();
    let Ok(input) = create_input(scenario, &secrets) else {
        return json!({"failed": true});
    };
    let Ok(runtime) = runtime() else {
        return json!({"failed": true});
    };
    let outcome = runtime.block_on(transform_create_batch_request(
        input,
        |_settings, _region| ready(Ok::<Credentials, CredentialError>(held.clone())),
    ));
    match outcome {
        Ok(signed) => json!({
            "failed": false,
            "method": signed.method,
            "url": signed.url,
            "headers": signed.headers,
            "data": signed.data,
        }),
        Err(_error) => json!({"failed": true}),
    }
}

fn reply_call(scenario: &Scenario) -> Value {
    let Ok(original) = original_request(scenario) else {
        return json!({"failed": true});
    };
    let body = &scenario.response_body;
    let clock = scripted_clock(&scenario.clock);
    match create_batch_reply(body.as_bytes(), original, &clock) {
        Ok(reply) => json!({"failed": false, "reply": reply}),
        Err(_error) => json!({"failed": true}),
    }
}

fn original_request(scenario: &Scenario) -> Result<OriginalBatchRequest, Box<HelperError>> {
    scenario.original_batch_request.as_ref().map_or_else(
        || Ok(OriginalBatchRequest::default()),
        |value| serde_json::from_value(value.clone()).or_value_error(),
    )
}

fn scripted_clock(values: &[i64]) -> impl Fn() -> i64 + '_ {
    let position = Cell::new(0_usize);
    move || {
        let index = position.get();
        position.set(index.saturating_add(1));
        values.get(index).copied().unwrap_or_default()
    }
}

fn values_operation(
    input: &Input,
    project: fn(&Json) -> Result<Value, Box<HelperError>>,
) -> HelperResult {
    let mut values = Vec::with_capacity(input.values.len());
    for value in &input.values {
        let decoded: Json = serde_json::from_value(value.clone()).or_value_error()?;
        values.push(project(&decoded)?);
    }
    Ok(json!({"values": values}))
}
