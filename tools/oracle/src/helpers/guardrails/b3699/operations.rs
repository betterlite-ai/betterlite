use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_config::sections::content_filter_inputs::DEFAULT_RECURSE_DEPTH;
use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_filter::competitor_intent_airline::AirlineCompetitorIntentChecker;
use betterlite_guardrails::content_filter::competitor_intent_airline::settings::AirlineSettings;
use betterlite_guardrails::content_filter::competitor_intent_base::configuration::Settings as CompetitorSettings;
use betterlite_guardrails::content_filter_actions::filter_single_text;
use betterlite_guardrails::content_filter_inputs::arguments::{
    ArgumentFilter, scan_tool_call_arguments,
};
use betterlite_guardrails::content_filter_inputs::competitor::{
    CompetitorMessageTemplates, apply_competitor_intent_policy,
};
use betterlite_guardrails::content_filter_inputs::error::ArgumentFilterError;
use betterlite_guardrails::content_filter_inputs::images::{
    ImageDescriptionCompletion, ImageDescriptionReply, ImageDescriptionRequest,
    ImageDescriptionResponse, ImageScan, process_images,
};
use betterlite_guardrails::content_filter_inputs::mcp::scan_mcp_tool_call_arguments;
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::projection;
use super::projection::output;
use super::{
    CompetitorConfigInput, CompetitorInput, Fixture, ImagesInput, McpInput, ToolCallsInput,
};
use super::{failure, guardrail, runtime};

struct ScriptedDescriptions(String);

#[async_trait]
impl ImageDescriptionCompletion for ScriptedDescriptions {
    type Error = std::io::Error;

    async fn describe(
        &self,
        _request: &ImageDescriptionRequest,
    ) -> Result<ImageDescriptionResponse, Self::Error> {
        Ok(ImageDescriptionResponse {
            choices: vec![ImageDescriptionReply {
                content: Some(self.0.clone()),
            }],
        })
    }
}

fn filtered_error(outcome: Result<(), ArgumentFilterError>) -> Value {
    match outcome {
        Ok(()) => Value::Null,
        Err(failure) => projection::failure(&failure.into_failure()),
    }
}

fn scan_images(
    input: &ImagesInput,
) -> Result<(Vec<ContentFilterDetection>, Value), Box<HelperError>> {
    let completion = ScriptedDescriptions(input.image_description.clone());
    let mut detections = Vec::new();
    let fixture = Fixture::new(&input.patterns, &input.blocked_words)?;
    let source = fixture.source();
    let error = match runtime()?.block_on(process_images(
        &ImageScan {
            images: &input.images,
            image_model: Some(input.image_model.as_str()),
            source: &source,
        },
        Some(&completion),
        &mut detections,
    )) {
        Ok(()) => Value::Null,
        Err(failure) => projection::failure(&failure.into_failure()),
    };
    Ok((detections, error))
}

pub(super) fn run_images(input: &ImagesInput) -> HelperResult {
    let (detections, error) = scan_images(input)?;
    Ok(output(&[], &detections, &[], &BTreeMap::new(), error))
}

fn airline_settings(config: &CompetitorConfigInput) -> AirlineSettings {
    AirlineSettings {
        base: CompetitorSettings {
            brand_self: config.brand_self.clone(),
            competitors: config.competitors.clone(),
            competitor_aliases: BTreeMap::new(),
            locations: config.locations.clone(),
            policy: config.policy.clone(),
            threshold_high: config.threshold_high.unwrap_or(0.70_f64),
            threshold_medium: config.threshold_medium.unwrap_or(0.45_f64),
            threshold_low: config.threshold_low.unwrap_or(0.30_f64),
            reframe_message_template: config.reframe_message_template.clone(),
            refuse_message_template: config.refuse_message_template.clone(),
            comparison_words: CompetitorSettings::default().comparison_words,
            domain_words: Vec::new(),
        },
        other_meaning_signals: Vec::new(),
        competitor_signals: Vec::new(),
        other_meaning_anchors: Vec::new(),
        explicit_competitor_marker: None,
        explicit_other_meaning_marker: None,
    }
}

pub(super) fn run_competitor(input: &CompetitorInput) -> HelperResult {
    let fixture = Fixture::new(&input.patterns, &input.blocked_words)?;
    let source = fixture.source();
    let settings = airline_settings(&input.config);
    let checker = AirlineCompetitorIntentChecker::new(&settings)
        .map_err(|issue| failure("CompetitorError", issue.to_string()))?;
    let result = checker.run(&input.text);
    let request_data = BTreeMap::from([(
        "model".to_owned(),
        GuardrailValue::Text(input.model.clone()),
    )]);
    let mut detections = Vec::new();
    let mut error = policy_error(&settings, &result, &request_data, &mut detections);
    // LiteLLM writes the filtered texts back only after its loop, so a raised policy leaves the input text
    let mut texts = vec![input.text.clone()];
    if error.is_null() {
        match filter_single_text(&source, &input.text, Some(&mut detections)) {
            Ok(filtered) => texts = vec![filtered],
            Err(issue) => error = projection::action_failure(&issue),
        }
    }
    Ok(output(&texts, &detections, &[], &BTreeMap::new(), error))
}

fn policy_error(
    settings: &AirlineSettings,
    result: &CompetitorIntentResult,
    request_data: &BTreeMap<String, GuardrailValue>,
    detections: &mut Vec<ContentFilterDetection>,
) -> Value {
    if result.intent.as_deref() == Some("other") {
        return Value::Null;
    }
    let templates = CompetitorMessageTemplates {
        refuse: settings.base.refuse_message_template.as_deref(),
        reframe: settings.base.reframe_message_template.as_deref(),
    };
    match apply_competitor_intent_policy(
        &guardrail("pre_call"),
        result,
        &templates,
        request_data,
        detections,
    ) {
        Ok(()) => Value::Null,
        Err(failure) => projection::failure(&failure),
    }
}

pub(super) fn run_tool_calls(input: &ToolCallsInput) -> HelperResult {
    let mut calls = input.tool_calls.clone();
    let fixture = Fixture::new(&input.patterns, &input.blocked_words)?;
    let source = fixture.source();
    let mut detections = Vec::new();
    let error = filtered_error(scan_tool_call_arguments(
        &ArgumentFilter {
            source: &source,
            max_depth: DEFAULT_RECURSE_DEPTH,
        },
        &mut calls,
        &mut detections,
    ));
    Ok(output(&[], &detections, &calls, &BTreeMap::new(), error))
}

pub(super) fn run_mcp(input: &McpInput) -> HelperResult {
    let fixture = Fixture::new(&input.patterns, &input.blocked_words)?;
    let source = fixture.source();
    let filter = ArgumentFilter {
        source: &source,
        max_depth: DEFAULT_RECURSE_DEPTH,
    };
    let mut request_data = input.request_data.clone();
    let mut detections = Vec::new();
    let error = filtered_error(scan_mcp_tool_call_arguments(
        &guardrail("pre_mcp_call"),
        &filter,
        None,
        &mut request_data,
        &mut detections,
    ));
    Ok(output(&[], &detections, &[], &request_data, error))
}
