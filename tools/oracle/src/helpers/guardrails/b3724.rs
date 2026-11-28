use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::CredentialSource;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock_guardrails::bedrock_transport::{BedrockGuardrail, BedrockGuardrailInit};
use betterlite_bedrock_guardrails::guardrail_checks::ChecksInput;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_guardrails::bedrock_apply::{ApiInvocation, make_bedrock_api_request};
use betterlite_guardrails::bedrock_checks::request::{ChecksEnvironment, ChecksScan};
use betterlite_guardrails::bedrock_message_selection::wire::{Message, ModelResponse};
use betterlite_guardrails::bedrock_records::{Response, Source};
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_models::catalog::BundledCatalog;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod replies;
mod tests;

use self::replies::Sent;

const RECORDS_KEY: &str = "standard_logging_guardrail_information";
const GUARDRAIL_NAME: &str = "bedrock-apply";
const REGION: &str = "us-west-2";
const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
const FIXED_NOW_SECONDS: u64 = 1_000_000;
// LiteLLM's BedrockGuardrail takes 0.5 for every check threshold when the config names none
const DEFAULT_THRESHOLD: Option<ConfidenceThreshold> = Some(ConfidenceThreshold(0.5));
const PROJECTED: [&str; 7] = [
    "guardrail_name",
    "guardrail_provider",
    "guardrail_mode",
    "guardrail_status",
    "guardrail_response",
    "violation_categories",
    "guardrail_action",
];
const ENVIRONMENT: ChecksEnvironment<'static> = ChecksEnvironment {
    runtime_endpoint: None,
    region_name: None,
    region: None,
    bearer_token: None,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: Value,
    raw: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Args {
    source: Source,
    messages: Option<Vec<Message>>,
    response: Option<ModelResponse>,
    logging_event_type: Option<GuardrailEventHooks>,
    guardrail_identifier: Option<String>,
    guardrail_version: Option<String>,
    checks: Option<BTreeMap<String, GuardrailValue>>,
    chunk_budget_chars: u64,
    contextual_grounding: bool,
    disable_exception_on_block: bool,
    data: BTreeMap<String, GuardrailValue>,
    replies: Vec<Reply>,
}

#[derive(Clone, Copy, Default)]
struct Creds;

impl CredentialSource for Creds {
    fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        ready(Ok(Credentials {
            access_key_id: SecretString::from(ACCESS_KEY.to_owned()),
            secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
            session_token: None,
        }))
    }
}

fn guardrail(args: &Args) -> Result<BedrockGuardrail, Box<HelperError>> {
    let mut params = BTreeMap::new();
    let pairs = [
        ("aws_access_key_id", ACCESS_KEY),
        ("aws_secret_access_key", SECRET_KEY),
        ("aws_region_name", REGION),
    ];
    for (key, value) in pairs {
        params.insert(key.to_owned(), GuardrailValue::Text(value.to_owned()));
    }
    BedrockGuardrail::__init__(BedrockGuardrailInit {
        guardrail_identifier: args.guardrail_identifier.clone(),
        guardrail_version: args.guardrail_version.clone(),
        disable_exception_on_block: Some(args.disable_exception_on_block),
        checks: args.checks.clone().map(ChecksInput::Mapping),
        content_filter_threshold: DEFAULT_THRESHOLD,
        prompt_attack_threshold: DEFAULT_THRESHOLD,
        pii_confidence_threshold: DEFAULT_THRESHOLD,
        chunk_budget_chars: args.chunk_budget_chars,
        contextual_grounding: args.contextual_grounding,
        latest_role_message_only: false,
        mask_request_content: false,
        mask_response_content: false,
        buffer_until_moderated: None,
        streaming_sampling_rate: None,
        streaming_end_of_stream_only: None,
        buffer_release_on_scan: None,
        optional_params: OpaqueParams(params),
        timeout: None,
    })
    .or_config_error()
}

fn base_settings() -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(GUARDRAIL_NAME.to_owned()),
        supported_event_hooks: None,
        event_hook: None,
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches {
            use_native_during_call_hook: false,
            use_native_lifecycle_hooks: false,
            records_own_information: false,
            uses_apply_guardrail_interface: false,
            deny_text_on_error: false,
        },
    }
}

fn helper_now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(FIXED_NOW_SECONDS))
        .unwrap_or(UNIX_EPOCH)
}

async fn run_one(
    args: &Args,
    guardrail: &BedrockGuardrail,
    data: &mut BTreeMap<String, GuardrailValue>,
    transport: &Sent,
) -> Result<Response, GuardrailFailure> {
    let base = GuardrailBase {
        settings: base_settings(),
        mode: None,
    };
    let creds = Creds;
    let catalog = BundledCatalog::default().get();
    let mut invocation = ApiInvocation {
        guardrail,
        base: &base,
        request_data: data,
        log_target: None,
        environment: ENVIRONMENT,
        catalog: catalog.as_ref(),
        now: helper_now,
    };
    let scan = ChecksScan {
        source: args.source,
        messages: args.messages.as_deref(),
        response: args.response.as_ref(),
        logging_event_type: args.logging_event_type,
    };
    make_bedrock_api_request(&mut invocation, scan, &creds, transport).await
}

fn outcome_value(outcome: Result<Response, GuardrailFailure>) -> Result<Value, Box<HelperError>> {
    match outcome {
        Ok(response) => Ok(json!({
            "kind": "allowed",
            "result": serde_json::to_value(response).or_value_error()?,
        })),
        Err(GuardrailFailure::HttpObject { status, detail }) => Ok(json!({
            "kind": "rejected",
            "status_code": status,
            "detail": serde_json::to_value(detail).or_value_error()?,
        })),
        Err(GuardrailFailure::Http { status, detail }) => Ok(json!({
            "kind": "rejected",
            "status_code": status,
            "detail": detail,
        })),
        Err(GuardrailFailure::ModifyResponse(failure)) => Ok(json!({
            "kind": "modify",
            "message": failure.message,
            "model": failure.model,
        })),
        Err(
            GuardrailFailure::GuardrailRaised { .. }
            | GuardrailFailure::BlockedPii { .. }
            | GuardrailFailure::SensitiveDataRoute(_)
            | GuardrailFailure::Other(_),
        ) => Err("the apply call failed unexpectedly".to_owned()).or_value_error(),
    }
}

fn records_value(data: &BTreeMap<String, GuardrailValue>) -> Result<Value, Box<HelperError>> {
    let Some(GuardrailValue::Object(metadata)) = data.get("metadata") else {
        return Ok(Value::Array(Vec::new()));
    };
    let Some(records) = metadata.get(RECORDS_KEY) else {
        return Ok(Value::Array(Vec::new()));
    };
    let encoded = serde_json::to_value(records).or_value_error()?;
    let entries = encoded.as_array().cloned().unwrap_or_default();
    Ok(Value::Array(entries.iter().map(project_record).collect()))
}

fn project_record(record: &Value) -> Value {
    let mut fields = serde_json::Map::new();
    for key in PROJECTED {
        let value = record.get(key).cloned().unwrap_or(Value::Null);
        fields.insert(key.to_owned(), value);
    }
    Value::Object(fields)
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Args;

    fn run(input: &Self::Input) -> HelperResult {
        let transport = Sent::new(&input.replies)?;
        let guardrail = guardrail(input)?;
        let mut data = input.data.clone();
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let outcome = runtime.block_on(run_one(input, &guardrail, &mut data, &transport));
        Ok(json!({
            "outcomes": [outcome_value(outcome)?],
            "records": records_value(&data)?,
            "requests": serde_json::to_value(transport.requests()).or_value_error()?,
        }))
    }
}
