use std::io;

use async_trait::async_trait;
use betterlite_config::sections::content_filter_records::{BlockedWord, ContentFilterPattern};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_guardrails::content_filter_guard::ContentFilterGuardrail;
use betterlite_guardrails::content_filter_init::args::ContentFilterArgs;
use betterlite_guardrails::content_filter_inputs::images::{
    ImageDescriptionCompletion, ImageDescriptionRequest, ImageDescriptionResponse,
};
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod projection;

pub(crate) const GUARDRAIL_NAME: &str = "content-filter";
const DEFAULT_EVENT_HOOK: &str = "pre_call";

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    builds: Vec<BuildInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildInput {
    args: ArgsInput,
    apply: Option<ApplyInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArgsInput {
    patterns: Vec<ContentFilterPattern>,
    blocked_words: Vec<BlockedWord>,
    event_hook: Option<LitellmParamsMode>,
    default_on: bool,
    guardrail_id: Option<String>,
    policy_template: Option<String>,
    image_model: Option<String>,
    competitor_intent_config: Option<OpaqueParams>,
    severity_threshold: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApplyInput {
    texts: Vec<String>,
    tool_calls: Option<Vec<GuardrailValue>>,
    input_type: GuardrailInputType,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        let mut builds = Vec::with_capacity(input.builds.len());
        for build in &input.builds {
            builds.push(runtime.block_on(build_output(build))?);
        }
        Ok(json!({"builds": builds}))
    }
}

async fn build_output(input: &BuildInput) -> HelperResult {
    let guardrail = ContentFilterGuardrail::<NoRouter>::new(content_filter_args(&input.args), None)
        .map_err(|error| failure("BuildError", error.to_string()))?;
    let build = projection::build(&guardrail)?;
    let applied = match &input.apply {
        Some(apply) => projection::apply(&guardrail, apply).await?,
        None => serde_json::Value::Null,
    };
    Ok(json!({"build": build, "apply": applied}))
}

fn content_filter_args(input: &ArgsInput) -> ContentFilterArgs {
    ContentFilterArgs {
        name: GUARDRAIL_NAME.to_owned(),
        guardrail_id: input.guardrail_id.clone(),
        policy_template: input.policy_template.clone(),
        patterns: Some(input.patterns.clone()),
        blocked_words: Some(input.blocked_words.clone()),
        blocked_words_file: None,
        event_hook: input
            .event_hook
            .clone()
            .unwrap_or_else(|| LitellmParamsMode::One(DEFAULT_EVENT_HOOK.to_owned())),
        default_on: input.default_on,
        categories: None,
        severity_threshold: input.severity_threshold.clone(),
        router_present: false,
        image_model: input.image_model.clone(),
        competitor_intent_config: input.competitor_intent_config.clone(),
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

// The cases never scan images, so the router the class holds is never asked for a description
struct NoRouter;

#[async_trait]
impl ImageDescriptionCompletion for NoRouter {
    type Error = io::Error;

    async fn describe(
        &self,
        _request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        Err(io::Error::other("the case supplies no router"))
    }
}
