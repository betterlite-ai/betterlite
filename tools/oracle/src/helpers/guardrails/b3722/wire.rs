use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::bedrock_records::{Response, Usage};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) bedrock_decisions: Command,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Command {
    ShouldRaise(Box<ResponseArgs>),
    BlockedAssessments(Box<ResponseArgs>),
    ViolationNames(Box<ResponseArgs>),
    HasException(Box<HasExceptionArgs>),
    Status(Box<StatusArgs>),
    TracingDetail(Box<TracingArgs>),
    HttpException(Box<HttpExceptionArgs>),
    LogAttempt(Box<LogAttemptArgs>),
    LogSuccess(Box<LogSuccessArgs>),
    LogFailure(Box<LogFailureArgs>),
}

#[derive(Debug, Deserialize)]
pub(super) struct ResponseArgs {
    pub response: Response,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum View {
    Json { payload: Json },
    Content { body: String },
    Text { text: String },
}

#[derive(Debug, Deserialize)]
pub(super) struct HasExceptionArgs {
    pub view: View,
}

#[derive(Debug, Deserialize)]
pub(super) struct StatusArgs {
    pub status_code: u16,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TracingArgs {
    pub response: Response,
    pub aws_region_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct HttpExceptionArgs {
    pub response: Response,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub guardrail: GuardrailSetup,
}

#[derive(Debug, Deserialize)]
pub(super) struct GuardrailSetup {
    pub name: Option<String>,
    pub identifier: Option<String>,
    pub version: Option<String>,
    #[serde(default)]
    pub disable_exception_on_block: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct LogAttemptArgs {
    pub response: Response,
    pub status_code: u16,
    pub body: String,
    pub completed_chunk_usages: Vec<Usage>,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub guardrail: GuardrailSetup,
}

#[derive(Debug, Deserialize)]
pub(super) struct LogSuccessArgs {
    pub response: Response,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub guardrail: GuardrailSetup,
}

#[derive(Debug, Deserialize)]
pub(super) struct LogFailureArgs {
    pub detail: String,
    pub completed_chunk_usages: Vec<Usage>,
    pub request_data: BTreeMap<String, GuardrailValue>,
    pub guardrail: GuardrailSetup,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum FailureView {
    Http {
        status: u16,
        detail: IndexMap<String, GuardrailValue>,
    },
    Modify {
        message: String,
        model: String,
        request_data: BTreeMap<String, GuardrailValue>,
        guardrail_name: Option<String>,
        detection_info: BTreeMap<String, GuardrailValue>,
    },
}
