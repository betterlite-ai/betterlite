use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::content_filter_inputs::DEFAULT_RECURSE_DEPTH;
use betterlite_config::sections::content_filter_records::BlockedWord;
use betterlite_config::sections::content_filter_records::ContentFilterDetection;
use betterlite_config::sections::content_filter_records::ContentFilterGuardrailConfig;
use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentResult;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::content_filter::detection_metrics::count_masked_entities;
use crate::content_filter::risk_scoring::GuardrailStatus;
use crate::content_filter_actions::ContentFilterSource;
use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_actions::filter_single_text;
use crate::content_filter_inputs::arguments::ArgumentFilter;
use crate::content_filter_inputs::arguments::scan_tool_call_arguments;
use crate::content_filter_inputs::competitor::CompetitorMessageTemplates;
use crate::content_filter_inputs::competitor::apply_competitor_intent_policy;
use crate::content_filter_inputs::error::ArgumentFilterError;
use crate::content_filter_inputs::error::ImageProcessingError;
use crate::content_filter_inputs::images::ImageDescriptionCompletion;
use crate::content_filter_inputs::images::ImageScan;
use crate::content_filter_inputs::images::process_images;
use crate::content_filter_inputs::mcp::scan_mcp_tool_call_arguments;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::inputs::GuardrailLogTarget;

use super::CompetitorIntentChecker;
use super::ContentFilterGuardrail;
use super::SUPPORTED_EVENT_HOOKS;
use super::record::RecordTrace;

const CONFIG_MODEL: &str = "LitellmContentFilterGuardrailConfigModel";

impl<R: ImageDescriptionCompletion> GuardrailContract for ContentFilterGuardrail<R> {
    async fn apply_guardrail(
        &self,
        mut inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let started = SystemTime::now();
        let mut detections = Vec::new();
        let outcome = self
            .scan(&mut inputs, request_data, input_type, &mut detections)
            .await;
        let mut masked_entity_count = BTreeMap::new();
        if outcome.is_ok() {
            count_masked_entities(&detections, &mut masked_entity_count);
        }
        let exception = outcome_exception(&outcome);
        let trace = RecordTrace {
            detections: &detections,
            masked_entity_count: &masked_entity_count,
            status: outcome_status(&outcome),
            exception: exception.as_deref(),
            started,
        };
        if let Err(error) = self.write_record(request_data, logging_obj, &trace) {
            tracing::debug!("ContentFilterGuardrail: guardrail record was not written: {error}");
        }
        match outcome {
            Ok(()) => Ok(inputs),
            Err(failure) => Err(failure),
        }
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        Some(GuardrailValue::Object(BTreeMap::from([
            (
                "model".to_owned(),
                GuardrailValue::Text(CONFIG_MODEL.to_owned()),
            ),
            (
                "ui_friendly_name".to_owned(),
                GuardrailValue::Text(ContentFilterGuardrailConfig::UI_FRIENDLY_NAME.to_owned()),
            ),
        ])))
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}

impl<R: ImageDescriptionCompletion> ContentFilterGuardrail<R> {
    async fn scan(
        &self,
        inputs: &mut GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        detections: &mut Vec<ContentFilterDetection>,
    ) -> Result<(), GuardrailFailure> {
        let blocked_words = self.blocked_word_list();
        let source = self.content_filter_source(&blocked_words);
        self.process_input_images(inputs, &source, detections)
            .await?;
        self.filter_texts(inputs, &source, request_data, detections)?;
        let filter = ArgumentFilter {
            source: &source,
            max_depth: DEFAULT_RECURSE_DEPTH,
        };
        scan_tool_call_inputs(&filter, inputs, detections)?;
        if input_type == GuardrailInputType::Request {
            // the logging target has no call_type, so LiteLLM's call_mcp_tool check has nothing to read
            scan_mcp_tool_call_arguments(&self.base, &filter, None, request_data, detections)
                .map_err(ArgumentFilterError::into_failure)?;
        }
        Ok(())
    }

    async fn process_input_images(
        &self,
        inputs: &GuardrailApiInputs,
        source: &ContentFilterSource<'_>,
        detections: &mut Vec<ContentFilterDetection>,
    ) -> Result<(), GuardrailFailure> {
        let scan = ImageScan {
            images: inputs.images.as_deref().unwrap_or_default(),
            image_model: self.image_model.as_deref(),
            source,
        };
        process_images(&scan, self.router.as_ref(), detections)
            .await
            .map_err(ImageProcessingError::into_failure)
    }

    fn filter_texts(
        &self,
        inputs: &mut GuardrailApiInputs,
        source: &ContentFilterSource<'_>,
        request_data: &BTreeMap<String, GuardrailValue>,
        detections: &mut Vec<ContentFilterDetection>,
    ) -> Result<(), GuardrailFailure> {
        let templates = CompetitorMessageTemplates {
            refuse: self.competitor_refuse_message_template.as_deref(),
            reframe: self.competitor_reframe_message_template.as_deref(),
        };
        let texts = inputs.texts.as_deref().unwrap_or_default();
        let mut processed = Vec::with_capacity(texts.len());
        for text in texts {
            self.apply_competitor_check(text, &templates, request_data, detections)?;
            let filtered =
                filter_single_text(source, text, Some(detections)).map_err(text_failure)?;
            processed.push(filtered);
        }
        inputs.texts = Some(processed);
        Ok(())
    }

    fn apply_competitor_check(
        &self,
        text: &str,
        templates: &CompetitorMessageTemplates<'_>,
        request_data: &BTreeMap<String, GuardrailValue>,
        detections: &mut Vec<ContentFilterDetection>,
    ) -> Result<(), GuardrailFailure> {
        let Some(checker) = &self.competitor_intent_checker else {
            return Ok(());
        };
        let Some(intent) = competitor_intent(checker, text) else {
            return Ok(());
        };
        if intent.intent.as_deref() != Some("other") {
            apply_competitor_intent_policy(
                &self.base,
                &intent,
                templates,
                request_data,
                detections,
            )?;
        }
        Ok(())
    }

    fn blocked_word_list(&self) -> Vec<BlockedWord> {
        self.blocked_words
            .iter()
            .map(|(keyword, (action, description))| BlockedWord {
                keyword: keyword.clone(),
                action: *action,
                description: description.clone(),
            })
            .collect()
    }

    fn content_filter_source<'source>(
        &'source self,
        blocked_words: &'source [BlockedWord],
    ) -> ContentFilterSource<'source> {
        ContentFilterSource {
            categories: &self.loaded_categories,
            patterns: &self.compiled_patterns,
            blocked_words,
            pattern_redaction_format: &self.pattern_redaction_format,
            keyword_redaction_tag: &self.keyword_redaction_tag,
        }
    }
}

// LiteLLM checks only a non-empty text, and both checkers already answer "other" for an empty one.
#[cfg_attr(test, mutants::skip)]
fn competitor_intent(
    checker: &CompetitorIntentChecker,
    text: &str,
) -> Option<CompetitorIntentResult> {
    if text.is_empty() {
        return None;
    }
    Some(checker.run(text))
}

fn scan_tool_call_inputs(
    filter: &ArgumentFilter<'_>,
    inputs: &mut GuardrailApiInputs,
    detections: &mut Vec<ContentFilterDetection>,
) -> Result<(), GuardrailFailure> {
    let Some(tool_calls) = inputs.tool_calls.as_mut() else {
        return Ok(());
    };
    scan_tool_call_arguments(filter, tool_calls, detections)
        .map_err(ArgumentFilterError::into_failure)
}

fn text_failure(error: ContentFilterActionError) -> GuardrailFailure {
    match error {
        ContentFilterActionError::Blocked { detail } => {
            ArgumentFilterError::Blocked { detail }.into_failure()
        }
        error @ (ContentFilterActionError::Matching(_)
        | ContentFilterActionError::Redaction(_)
        | ContentFilterActionError::Pattern(_)
        | ContentFilterActionError::Replacement(_)
        | ContentFilterActionError::Keyword(_)) => {
            ArgumentFilterError::Filter(error).into_failure()
        }
    }
}

const fn outcome_status(outcome: &Result<(), GuardrailFailure>) -> GuardrailStatus {
    match outcome {
        Ok(()) => GuardrailStatus::Success,
        Err(failure) => failure_status(failure),
    }
}

const fn failure_status(failure: &GuardrailFailure) -> GuardrailStatus {
    if is_http_failure(failure) {
        return GuardrailStatus::GuardrailIntervened;
    }
    GuardrailStatus::GuardrailFailedToRespond
}

fn outcome_exception(outcome: &Result<(), GuardrailFailure>) -> Option<String> {
    match outcome {
        Ok(()) => None,
        Err(failure) => exception_text(failure),
    }
}

// LiteLLM's except HTTPException sets no exception string, so only the other failures log their text
fn exception_text(failure: &GuardrailFailure) -> Option<String> {
    if is_http_failure(failure) {
        return None;
    }
    Some(failure.to_string())
}

const fn is_http_failure(failure: &GuardrailFailure) -> bool {
    matches!(
        failure,
        GuardrailFailure::Http { .. } | GuardrailFailure::HttpObject { .. }
    )
}
