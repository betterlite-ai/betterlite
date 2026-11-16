use betterlite_core::api::chat_response_stream_tools::fields::string_value;
use betterlite_core::shadow_evidence::chat_text;
use betterlite_core::shadow_evidence::cost::{PriceMap, call_cost};
use betterlite_core::shadow_evidence::judge_reply;
use betterlite_core::shadow_evidence::records::{CallFailure, JudgeVerdict};
use betterlite_logging::logging_privacy::internal::{self, Forwarding, Origin};
use betterlite_models::decisions::Json;
use betterlite_models::judge_primitives::content_text_extraction::extract_text_from_content;
use betterlite_models::judge_primitives::error::JudgePrimitivesError;
use betterlite_models::judge_primitives::judge_verdict_parsing::parse_json_verdict;
use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use crate::judge_targets::dispatcher::JudgeDispatcher;
use crate::judge_targets::error::JudgeCallError;
use crate::judge_targets::judge_acompletion;

use super::judge_prompt::{
    JUDGE_MAX_OUTPUT_TOKENS, PAIRWISE_JUDGE_SYSTEM_PROMPT, judge_user_prompt, tool_definitions_text,
};
use super::response_format::pairwise_judge_response_format;
use super::unmask::unmask_preference;
use super::{JudgeDeps, JudgeRequest, metadata_failure};

mod tests;

// split so the workspace typos dictionary keeps LiteLLM's spelling
const VERDICT_FAILURE_PREFIX: &str = concat!("un", "parseable judge verdict: ");

#[derive(Debug, thiserror::Error)]
pub enum JudgeVerdictError {
    #[error(transparent)]
    Parse(#[from] JudgePrimitivesError),
    #[error("confidence: field required")]
    MissingConfidence,
    #[error("confidence: input should be a number")]
    Confidence,
    #[error("confidence: input should be a number")]
    UnrepresentableConfidence(#[from] DecimalLexemeError),
    #[error("preference: field required")]
    MissingPreference,
    #[error("preference: input should be 'A', 'B' or 'tie'")]
    Preference,
}

struct ParsedVerdict {
    preference: String,
    confidence: Decimal,
}

pub async fn call_judge(
    request: &JudgeRequest<'_>,
    deps: &JudgeDeps<'_>,
) -> Result<JudgeVerdict, CallFailure> {
    let real_is_a = deps.coin.real_is_a();
    let response = judge_response(request, deps, real_is_a).await?;
    match parse_verdict(&reply_content(&response)) {
        Ok(parsed) => Ok(JudgeVerdict::parsed(
            unmask_preference(&parsed.preference, real_is_a),
            parsed.confidence,
            call_cost(&response, deps.prices),
        )),
        Err(error) => Err(verdict_failure(&response, &error, deps.prices)),
    }
}

async fn judge_response(
    request: &JudgeRequest<'_>,
    deps: &JudgeDeps<'_>,
    real_is_a: bool,
) -> Result<Json, CallFailure> {
    let judge_metadata = internal::forward(
        request.parent_metadata,
        Origin::ShadowEvalJudge,
        Forwarding::Detached,
    )
    .map_err(metadata_failure)?;
    let messages = judge_messages(request, real_is_a);
    let team_id = forwarded_team_id(request.parent_metadata);
    let router = deps.provider.judge_router();
    let dispatcher = JudgeDispatcher {
        router: router.as_deref(),
        sdk: deps.sdk,
        cache: deps.cache,
        context: deps.context,
    };
    judge_acompletion(
        &dispatcher,
        request.judge_model,
        messages,
        team_id.as_deref(),
        judge_params(judge_metadata),
    )
    .await
    .map_err(judge_call_failure)
}

fn judge_messages(request: &JudgeRequest<'_>, real_is_a: bool) -> Vec<Json> {
    let (response_a, response_b) = if real_is_a {
        (request.real_text, request.shadow_text)
    } else {
        (request.shadow_text, request.real_text)
    };
    let conversation = conversation_text(request.messages);
    let tool_text = tool_definitions_text(request.tools);
    let prompt = judge_user_prompt(&conversation, response_a, response_b, &tool_text);
    vec![
        message("system", PAIRWISE_JUDGE_SYSTEM_PROMPT),
        message("user", &prompt),
    ]
}

fn judge_params(judge_metadata: IndexMap<String, Json>) -> IndexMap<String, Json> {
    IndexMap::from([
        ("temperature".to_owned(), Json::Number(Number::from(0_u8))),
        (
            "max_tokens".to_owned(),
            Json::Number(Number::from(JUDGE_MAX_OUTPUT_TOKENS)),
        ),
        (
            "response_format".to_owned(),
            pairwise_judge_response_format(),
        ),
        ("metadata".to_owned(), Json::Object(judge_metadata)),
    ])
}

fn verdict_failure(
    response: &Json,
    error: &JudgeVerdictError,
    prices: &(dyn PriceMap + Sync),
) -> CallFailure {
    let shape = judge_reply::shape(response);
    CallFailure {
        error: format!("{VERDICT_FAILURE_PREFIX}{error}; {shape}"),
        cost: call_cost(response, prices),
        classifier_cost: Decimal::ZERO,
    }
}

#[must_use]
pub fn forwarded_team_id(metadata: &IndexMap<String, Json>) -> Option<String> {
    match metadata.get("user_api_key_team_id") {
        Some(Json::Text(team_id)) if !team_id.is_empty() => Some(team_id.clone()),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

fn message(role: &str, content: &str) -> Json {
    Json::Object(IndexMap::from([
        ("role".to_owned(), Json::Text(role.to_owned())),
        ("content".to_owned(), Json::Text(content.to_owned())),
    ]))
}

fn conversation_text(messages: &[Json]) -> String {
    let mut lines: Vec<String> = Vec::new();
    for turn in messages {
        let Some(content) =
            chat_text::field(turn, "content").filter(|value| !matches!(value, Json::Null))
        else {
            continue;
        };
        let role = chat_text::field(turn, "role")
            .map_or_else(|| "user".to_owned(), string_value)
            .to_uppercase();
        let text = extract_text_from_content(content);
        lines.push(format!("{role}: {text}"));
    }
    lines.join("\n")
}

fn reply_content(response: &Json) -> String {
    let choices = match chat_text::field(response, "choices") {
        Some(Json::List(choices)) => choices,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => {
            return String::new();
        }
    };
    let content = choices
        .first()
        .and_then(|choice| chat_text::field(choice, "message"))
        .and_then(|reply| chat_text::field(reply, "content"));
    match content {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}

// PairwiseVerdict's contract: both fields required, preference closed over the labels, extra keys ignored
fn parse_verdict(raw: &str) -> Result<ParsedVerdict, JudgeVerdictError> {
    let fields = parse_json_verdict(raw)?;
    let preference = match fields.get("preference") {
        None => return Err(JudgeVerdictError::MissingPreference),
        Some(Json::Text(text)) if matches!(text.as_str(), "A" | "B" | "tie") => text.clone(),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        ) => return Err(JudgeVerdictError::Preference),
    };
    let confidence = match fields.get("confidence") {
        None => return Err(JudgeVerdictError::MissingConfidence),
        Some(Json::Number(amount)) => parse_decimal_lexeme(&amount.to_string())?,
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            return Err(JudgeVerdictError::Confidence);
        }
    };
    Ok(ParsedVerdict {
        preference,
        confidence,
    })
}

fn judge_call_failure(error: JudgeCallError) -> CallFailure {
    let text = match error {
        JudgeCallError::Nothing { .. } => error.to_string(),
        JudgeCallError::Dispatch { source, .. } => source.message,
    };
    CallFailure {
        error: format!("judge call failed: {text}"),
        cost: Decimal::ZERO,
        classifier_cost: Decimal::ZERO,
    }
}
