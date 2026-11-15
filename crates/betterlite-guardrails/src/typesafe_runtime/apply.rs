use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::Json;

use crate::generic_guardrail_contracts::GuardrailMessage;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::typesafe::typesafe_api_call::{JevSystemOneResponse, call_systemone};
use crate::typesafe::typesafe_candidate_exchanges::candidate_exchanges;
use crate::typesafe::typesafe_state_building::{TypeSafeState, build_state};
use crate::typesafe::{DROPPED_RESULT_TEXT, TypeSafeGuardrail};

use super::TypeSafeGuard;
use super::records::{Timing, record_compacted, record_unavailable};

pub(super) struct CompactionCounts {
    pub(super) chars_removed: i64,
    pub(super) exchanges_dropped: i64,
    pub(super) exchanges_evaluated: i64,
}

struct Compaction {
    counts: CompactionCounts,
    indices: BTreeSet<usize>,
}

struct DropPlan {
    exchanges: usize,
    indices: BTreeSet<usize>,
}

enum Evaluated {
    Answers(JevSystemOneResponse, Timing),
    Unavailable(Timing),
}

pub(super) async fn compact(
    guard: &TypeSafeGuard,
    inputs: GuardrailApiInputs,
    rows: &[Json],
    request_data: &mut BTreeMap<String, GuardrailValue>,
    log_target: Option<&mut GuardrailLogTarget>,
) -> Result<GuardrailApiInputs, GuardrailFailure> {
    if rows.is_empty() || rows.iter().any(|row| row.as_object().is_none()) {
        return Ok(inputs);
    }
    let candidates = candidate_exchanges(&guard.runtime, rows);
    if candidates.is_empty() {
        tracing::debug!("TypeSafe: no completed tool exchanges eligible for evaluation");
        return Ok(inputs);
    }
    let state = build_state(&guard.runtime, rows, &candidates);
    let (response, timing) = match evaluate(guard, &state, candidates.len()).await? {
        Evaluated::Answers(response, timing) => (response, timing),
        Evaluated::Unavailable(timing) => {
            record_unavailable(guard, request_data, log_target, &timing);
            return Ok(inputs);
        }
    };
    let Some(compaction) = compact_answers(guard, rows, &candidates, &response) else {
        return Ok(inputs);
    };
    record_compacted(guard, request_data, log_target, &compaction.counts, &timing);
    if let Some(messages) = inputs.structured_messages.as_mut() {
        rewrite_contents(messages, &compaction.indices);
    }
    Ok(inputs)
}

async fn evaluate(
    guard: &TypeSafeGuard,
    state: &TypeSafeState,
    exchanges: usize,
) -> Result<Evaluated, GuardrailFailure> {
    let question_ids: Vec<String> = (0..exchanges)
        .map(|ordinal| format!("e{ordinal}"))
        .collect();
    let started = Instant::now();
    let clock_started = (guard.clock)();
    let response = call_systemone(&guard.runtime, state, &question_ids).await;
    let duration = started.elapsed();
    let timing = Timing {
        started: clock_started,
        ended: (guard.clock)(),
        duration,
    };
    match response {
        Ok(Some(response)) => Ok(Evaluated::Answers(response, timing)),
        Ok(None) => Ok(Evaluated::Unavailable(timing)),
        Err(error) => Err(GuardrailFailure::from(error)),
    }
}

fn compact_answers(
    guard: &TypeSafeGuard,
    rows: &[Json],
    candidates: &[Vec<usize>],
    response: &JevSystemOneResponse,
) -> Option<Compaction> {
    let threshold = guard.runtime.settings.relevance_threshold.0;
    let plan = plan_drops(response, candidates, rows, threshold);
    if plan.indices.is_empty() {
        tracing::debug!("TypeSafe: all evaluated exchanges still relevant; request unchanged");
        return None;
    }
    let chars_removed = removed_chars(&guard.runtime, rows, &plan.indices);
    tracing::info!(
        "TypeSafe: evaluated {} tool exchange(s), dropped {}, ~{} chars removed",
        candidates.len(),
        plan.exchanges,
        chars_removed
    );
    let counts = CompactionCounts {
        chars_removed,
        exchanges_dropped: i64::try_from(plan.exchanges).unwrap_or(i64::MAX),
        exchanges_evaluated: i64::try_from(candidates.len()).unwrap_or(i64::MAX),
    };
    Some(Compaction {
        counts,
        indices: plan.indices,
    })
}

fn plan_drops(
    response: &JevSystemOneResponse,
    candidates: &[Vec<usize>],
    rows: &[Json],
    threshold: f64,
) -> DropPlan {
    let mut plan = DropPlan {
        exchanges: 0,
        indices: BTreeSet::new(),
    };
    for (ordinal, group) in candidates.iter().enumerate() {
        let Some(answer) = response.answers.get(&format!("e{ordinal}")) else {
            continue;
        };
        if answer.noul >= threshold {
            continue;
        }
        plan.exchanges = plan.exchanges.saturating_add(1);
        for index in group.iter().skip(1) {
            if is_tool_result(rows, *index) {
                plan.indices.insert(*index);
            }
        }
    }
    plan
}

fn is_tool_result(rows: &[Json], index: usize) -> bool {
    let role = rows
        .get(index)
        .and_then(Json::as_object)
        .and_then(|fields| fields.get("role"))
        .and_then(Json::as_str);
    matches!(role, Some("tool" | "function"))
}

fn removed_chars(guardrail: &TypeSafeGuardrail, rows: &[Json], dropped: &BTreeSet<usize>) -> i64 {
    let marker = i64::try_from(DROPPED_RESULT_TEXT.chars().count()).unwrap_or(i64::MAX);
    let mut removed = 0_i64;
    for index in dropped {
        let content = rows
            .get(*index)
            .and_then(Json::as_object)
            .and_then(|fields| fields.get("content"));
        let text = (guardrail.content_to_text)(content);
        let length = i64::try_from(text.chars().count()).unwrap_or(i64::MAX);
        removed = removed.saturating_add(length).saturating_sub(marker);
    }
    removed
}

fn rewrite_contents(messages: &mut [GuardrailMessage], dropped: &BTreeSet<usize>) {
    for index in dropped {
        let Some(message) = messages.get_mut(*index) else {
            continue;
        };
        message.extra.insert(
            "content".to_owned(),
            GuardrailValue::Text(DROPPED_RESULT_TEXT.to_owned()),
        );
    }
}
