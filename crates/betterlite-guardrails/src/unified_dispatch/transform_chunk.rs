use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use indexmap::IndexMap;

use crate::unified_dispatch::emit::{HttpErrorDetail, StreamHttpError};
use crate::unified_dispatch::stream_items::{response_chunk, stream_choice};

pub struct TransformChunkInput<'input> {
    pub reference_chunk: &'input GuardrailValue,
    pub mutated_text_per_choice: &'input IndexMap<i64, String>,
    pub emitted_text_per_choice: &'input mut IndexMap<i64, String>,
    pub holdback_per_choice: &'input IndexMap<i64, i64>,
    pub finish_reason_per_choice: &'input IndexMap<i64, Option<String>>,
    pub held_chars_per_choice: &'input mut IndexMap<i64, i64>,
    pub is_final: bool,
}

pub fn build_transform_chunk(
    input: TransformChunkInput<'_>,
) -> Result<Option<GuardrailValue>, StreamHttpError> {
    let TransformChunkInput {
        reference_chunk,
        mutated_text_per_choice,
        emitted_text_per_choice,
        holdback_per_choice,
        finish_reason_per_choice,
        held_chars_per_choice,
        is_final,
    } = input;
    if mutated_text_per_choice.is_empty() {
        return Ok(terminator_chunk(
            reference_chunk,
            finish_reason_per_choice,
            is_final,
        ));
    }
    let deltas = round_deltas(
        mutated_text_per_choice,
        emitted_text_per_choice,
        holdback_per_choice,
        held_chars_per_choice,
        is_final,
    )?;
    let choices = round_choices(
        &deltas,
        emitted_text_per_choice,
        finish_reason_per_choice,
        is_final,
    );
    if choices.is_empty() {
        return Ok(None);
    }
    add_deltas(mutated_text_per_choice, &deltas, emitted_text_per_choice);
    Ok(Some(response_chunk(reference_chunk, choices)))
}

fn underflow(index: i64, already_chars: usize) -> StreamHttpError {
    let message = format!(
        "Guardrail streaming transform for choice {index} is not a forward extension of the {already_chars} chars already streamed to the client (it is shorter than, or rewrites, the emitted prefix); emitted bytes cannot be retracted. Withhold recent output via stream_holdback_chars before rewriting it."
    );
    let detail = BTreeMap::from([
        (
            "error".to_owned(),
            GuardrailValue::Text("stream_transform_underflow".to_owned()),
        ),
        ("message".to_owned(), GuardrailValue::Text(message)),
    ]);
    StreamHttpError {
        status_code: 400,
        detail: HttpErrorDetail::Object(detail),
    }
}

// on the final round a deferred finish_reason still reaches the client when the guardrail sent no text
fn terminator_chunk(
    reference: &GuardrailValue,
    reasons: &IndexMap<i64, Option<String>>,
    is_final: bool,
) -> Option<GuardrailValue> {
    if !is_final {
        return None;
    }
    let mut choices = Vec::new();
    for (index, reason) in reasons {
        let Some(given) = reason else {
            continue;
        };
        let mapped = if given.is_empty() {
            None
        } else {
            Some(map_finish_reason(given).reason.as_str().to_owned())
        };
        choices.push(stream_choice(
            *index,
            GuardrailValue::Text(String::new()),
            GuardrailValue::Null,
            GuardrailValue::Null,
            mapped,
        ));
    }
    if choices.is_empty() {
        return None;
    }
    Some(response_chunk(reference, choices))
}

fn round_deltas(
    mutated: &IndexMap<i64, String>,
    emitted: &IndexMap<i64, String>,
    holdback: &IndexMap<i64, i64>,
    held: &mut IndexMap<i64, i64>,
    is_final: bool,
) -> Result<IndexMap<i64, String>, StreamHttpError> {
    let mut deltas = IndexMap::new();
    for (index, text) in mutated {
        let already = emitted.get(index).map_or("", String::as_str);
        let already_chars = already.chars().count();
        let text_chars = text.chars().count();
        if !text.starts_with(already) {
            return Err(underflow(*index, already_chars));
        }
        let window = if is_final {
            0
        } else {
            holdback.get(index).copied().unwrap_or(0).max(0)
        };
        let withheld = usize::try_from(window).unwrap_or(usize::MAX);
        let end = already_chars.max(text_chars.saturating_sub(withheld));
        let delta = text
            .chars()
            .skip(already_chars)
            .take(end.saturating_sub(already_chars))
            .collect();
        deltas.insert(*index, delta);
        held.insert(
            *index,
            i64::try_from(text_chars.saturating_sub(end)).unwrap_or(i64::MAX),
        );
    }
    Ok(deltas)
}

fn round_choices(
    deltas: &IndexMap<i64, String>,
    emitted: &IndexMap<i64, String>,
    reasons: &IndexMap<i64, Option<String>>,
    is_final: bool,
) -> Vec<GuardrailValue> {
    deltas
        .iter()
        .filter_map(|(index, delta)| choice_for(*index, delta, emitted, reasons, is_final))
        .collect()
}

fn choice_for(
    index: i64,
    delta: &str,
    emitted: &IndexMap<i64, String>,
    reasons: &IndexMap<i64, Option<String>>,
    is_final: bool,
) -> Option<GuardrailValue> {
    let stored = if is_final { reasons.get(&index) } else { None };
    // an empty reason still counts for the skip rule; StreamingChoices nulls it in the chunk
    if delta.is_empty() && !stored.is_some_and(Option::is_some) {
        return None;
    }
    Some(stream_choice(
        index,
        GuardrailValue::Text(delta.to_owned()),
        round_role(emitted.get(&index)),
        GuardrailValue::Null,
        round_reason(stored),
    ))
}

fn round_reason(stored: Option<&Option<String>>) -> Option<String> {
    let reason = stored.and_then(|given| given.as_ref().map(String::as_str));
    let given = reason.filter(|candidate| !candidate.is_empty())?;
    Some(map_finish_reason(given).reason.as_str().to_owned())
}

fn round_role(emitted: Option<&String>) -> GuardrailValue {
    match emitted {
        Some(already) if !already.is_empty() => GuardrailValue::Null,
        Some(_) | None => GuardrailValue::Text("assistant".to_owned()),
    }
}

fn add_deltas(
    mutated: &IndexMap<i64, String>,
    deltas: &IndexMap<i64, String>,
    emitted: &mut IndexMap<i64, String>,
) {
    for index in mutated.keys() {
        let delta = deltas.get(index).cloned().unwrap_or_default();
        emitted
            .entry(*index)
            .and_modify(|already| already.push_str(&delta))
            .or_insert(delta);
    }
}
