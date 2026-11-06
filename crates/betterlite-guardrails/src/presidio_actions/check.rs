use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::presidio_actions::PresidioAnonymizer;
use crate::presidio_actions::anonymize::{AnonymizeTextCall, anonymize_text};
use crate::presidio_actions::blocking::raise_if_blocked_entities;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::{
    PresidioPerRequestConfig, RequestData, append_guardrail_information,
};
use crate::presidio_analysis::wire::{AnalyzeOutcome, AnalyzeResponseItem};
use crate::presidio_analysis::{AnalyzerSettings, PresidioAnalyzer, RequestOverrides};

const PROVIDER_NAME: &str = "presidio";

pub struct CheckPiiCall<'call> {
    pub text: &'call str,
    pub output_parse_pii: bool,
    pub presidio_config: Option<&'call PresidioPerRequestConfig>,
    pub dynamic_params: &'call BTreeMap<String, GuardrailValue>,
    pub pii_entities_config: &'call BTreeMap<String, PiiAction>,
    pub request_data: &'call mut RequestData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailStatus {
    Success,
    GuardrailFailedToRespond,
}

#[derive(Debug, Serialize)]
struct GuardrailInformation<'record> {
    guardrail_name: &'record str,
    guardrail_provider: &'record str,
    guardrail_response: &'record RawValue,
    guardrail_status: GuardrailStatus,
    start_time: f64,
    end_time: f64,
    duration: f64,
    masked_entity_count: &'record IndexMap<String, usize>,
}

struct GuardrailLog<'log> {
    guardrail_name: &'log str,
    analyze_results: Option<&'log [AnalyzeResponseItem]>,
    error_message: Option<&'log str>,
    masked_entity_count: &'log IndexMap<String, usize>,
    start: SystemTime,
    end: SystemTime,
}

impl PresidioAnonymizer {
    pub async fn check_pii(
        &self,
        analyzer: &PresidioAnalyzer,
        mut call: CheckPiiCall<'_>,
    ) -> Result<String, PresidioActionsError> {
        let start = (self.clock)();
        let mut masked_entity_count: IndexMap<String, usize> = IndexMap::new();
        let mut analyze_results: Option<Vec<AnalyzeResponseItem>> = None;
        let outcome = self
            .run_check(
                analyzer,
                &mut call,
                &mut analyze_results,
                &mut masked_entity_count,
            )
            .await;
        let end = (self.clock)();
        let error_message = outcome.as_ref().err().map(ToString::to_string);
        let logged = log_guardrail_information(
            call.request_data,
            &GuardrailLog {
                guardrail_name: &analyzer.settings.guardrail_name,
                analyze_results: analyze_results.as_deref(),
                error_message: error_message.as_deref(),
                masked_entity_count: &masked_entity_count,
                start,
                end,
            },
        );
        match outcome {
            Ok(text) => {
                logged?;
                Ok(text)
            }
            Err(error) => {
                if let Err(log_error) = logged {
                    tracing::warn!(%log_error, "Presidio guardrail information was not recorded");
                }
                Err(error)
            }
        }
    }

    async fn run_check(
        &self,
        analyzer: &PresidioAnalyzer,
        call: &mut CheckPiiCall<'_>,
        analyze_results: &mut Option<Vec<AnalyzeResponseItem>>,
        masked_entity_count: &mut IndexMap<String, usize>,
    ) -> Result<String, PresidioActionsError> {
        if let Some(mock) = &analyzer.settings.mock_redacted_text {
            return Ok(mock.text.clone());
        }
        let overrides = RequestOverrides {
            language: call
                .presidio_config
                .and_then(|config| config.language.as_deref()),
            dynamic_params: call.dynamic_params,
        };
        let items = match analyzer
            .analyze_text(call.text, &overrides)
            .await
            .map_err(|source| PresidioActionsError::Analysis { source })?
        {
            AnalyzeOutcome::Items(items) => items,
            AnalyzeOutcome::Redacted(mock) => return Ok(mock.text),
        };
        let filtered = filter_results(&analyzer.settings, items);
        *analyze_results = Some(filtered.clone());
        raise_if_blocked_entities(
            &filtered,
            call.pii_entities_config,
            &analyzer.settings.guardrail_name,
        )?;
        anonymize_text(
            self,
            AnonymizeTextCall {
                text: call.text,
                analyze_results: &filtered,
                output_parse_pii: call.output_parse_pii,
                masked_entity_count,
                request_data: Some(&mut *call.request_data),
            },
        )
        .await
    }
}

#[must_use]
fn filter_results(
    settings: &AnalyzerSettings,
    results: Vec<AnalyzeResponseItem>,
) -> Vec<AnalyzeResponseItem> {
    if settings.score_thresholds.is_empty() && settings.entities_deny_list.is_empty() {
        return results;
    }
    results
        .into_iter()
        .filter(|item| kept_by_deny_list(settings, item))
        .filter(|item| meets_score_threshold(settings, item))
        .collect()
}

fn kept_by_deny_list(settings: &AnalyzerSettings, item: &AnalyzeResponseItem) -> bool {
    item.entity_type
        .as_ref()
        .is_none_or(|entity_type| !settings.entities_deny_list.contains(entity_type))
}

fn meets_score_threshold(settings: &AnalyzerSettings, item: &AnalyzeResponseItem) -> bool {
    let threshold = item
        .entity_type
        .as_ref()
        .and_then(|entity_type| settings.score_thresholds.get(entity_type))
        .or_else(|| settings.score_thresholds.get("ALL"));
    threshold.is_none_or(|minimum| item.score.is_some_and(|score| score >= *minimum))
}

fn log_guardrail_information(
    request_data: &mut RequestData,
    log: &GuardrailLog<'_>,
) -> Result<(), PresidioActionsError> {
    let status = if log.error_message.is_some() {
        GuardrailStatus::GuardrailFailedToRespond
    } else {
        GuardrailStatus::Success
    };
    let response = guardrail_response(log.error_message, log.analyze_results)?;
    let record = GuardrailInformation {
        guardrail_name: log.guardrail_name,
        guardrail_provider: PROVIDER_NAME,
        guardrail_response: &response,
        guardrail_status: status,
        start_time: epoch_seconds(log.start),
        end_time: epoch_seconds(log.end),
        duration: log
            .end
            .duration_since(log.start)
            .unwrap_or_default()
            .as_secs_f64(),
        masked_entity_count: log.masked_entity_count,
    };
    append_guardrail_information(request_data, &record)
}

fn guardrail_response(
    error_message: Option<&str>,
    analyze_results: Option<&[AnalyzeResponseItem]>,
) -> Result<Box<RawValue>, PresidioActionsError> {
    let encoded = match (error_message, analyze_results) {
        (Some(message), _) => serde_json::to_string(message)
            .map_err(|source| PresidioActionsError::Serialize { source })?,
        (None, Some(items)) => serde_json::to_string(items)
            .map_err(|source| PresidioActionsError::Serialize { source })?,
        (None, None) => "{}".to_owned(),
    };
    RawValue::from_string(encoded).map_err(|source| PresidioActionsError::Serialize { source })
}

fn epoch_seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}
