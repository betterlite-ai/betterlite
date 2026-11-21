use super::calls::EvaluationCall;
use super::error::HealthProbeError;
use super::params::HealthProbeParams;
use betterlite_models::decisions::systemone::DecisionQuestionMap;
use betterlite_models::decisions::{DecisionsJson, Extra, Json};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
pub const HEALTH_CHECK_QUESTION: &str = "Is the service reachable?";
const HOSTED_VLLM_PROVIDER: &str = "hosted_vllm";
const QUESTION_NAME: &str = "reachable";
const DEFAULT_STATE: &str = "health check";
#[derive(Clone, Debug, Serialize)]
pub struct EvaluationCallParams {
    pub model: String,
    pub state: DecisionsJson,
    pub questions: DecisionQuestionMap,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_llm_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(flatten)]
    pub extra: Extra,
    #[serde(skip)]
    pub api_key: Option<SecretString>,
}
#[must_use]
pub fn evaluation_questions(custom_llm_provider: &str) -> IndexMap<String, Json> {
    let mut question = IndexMap::new();
    question.insert(
        "type".to_owned(),
        Json::Text(question_type(custom_llm_provider)),
    );
    question.insert(
        "instructions".to_owned(),
        Json::Text(HEALTH_CHECK_QUESTION.to_owned()),
    );
    if custom_llm_provider == HOSTED_VLLM_PROVIDER {
        let mut criteria = IndexMap::new();
        criteria.insert("yes".to_owned(), Json::Null);
        criteria.insert("no".to_owned(), Json::Null);
        question.insert("criteria".to_owned(), Json::Object(criteria));
    }
    let mut questions = IndexMap::new();
    questions.insert(QUESTION_NAME.to_owned(), Json::Object(question));
    questions
}
fn question_type(custom_llm_provider: &str) -> String {
    if custom_llm_provider == HOSTED_VLLM_PROVIDER {
        "choice".to_owned()
    } else {
        "noul".to_owned()
    }
}
pub(crate) fn evaluation_call(
    params: &HealthProbeParams,
    prompt: Option<&str>,
    custom_llm_provider: &str,
) -> Result<EvaluationCall, HealthProbeError> {
    let mut payload = IndexMap::new();
    let state = prompt
        .filter(|text| !text.is_empty())
        .map_or_else(|| DEFAULT_STATE.to_owned(), str::to_owned);
    payload.insert("state".to_owned(), Json::Text(state));
    payload.insert(
        "questions".to_owned(),
        Json::Object(evaluation_questions(custom_llm_provider)),
    );
    // the filtered keys come last, so a state or questions key in the parameters wins
    for (key, value) in params.call_parameters(false) {
        payload.insert(key, value);
    }
    let mut validated = validate_payload(payload)?;
    validated.api_key = params.api_key.clone();
    Ok(EvaluationCall {
        params: validated,
        logging: params.litellm_logging_obj.clone(),
    })
}
fn validate_payload(
    mut payload: IndexMap<String, Json>,
) -> Result<EvaluationCallParams, HealthProbeError> {
    let model = take_text(&mut payload, "model")?;
    let state = take_state(&mut payload)?;
    let questions = take_questions(&mut payload)?;
    let api_base = take_optional_text(&mut payload, "api_base")?;
    let custom_llm_provider = take_optional_text(&mut payload, "custom_llm_provider")?;
    let timeout = take_timeout(&mut payload)?;
    let extra_headers = take_headers(&mut payload)?;
    let params = EvaluationCallParams {
        model,
        state,
        questions,
        api_base,
        custom_llm_provider,
        timeout,
        extra_headers,
        extra: payload,
        api_key: None,
    };
    Ok(params)
}
fn take_text(
    payload: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<String, HealthProbeError> {
    match payload.shift_remove(field) {
        Some(Json::Text(text)) => Ok(text),
        Some(_) => Err(invalid(field, "it must be a string")),
        None => Err(invalid(field, "it is required")),
    }
}
fn take_optional_text(
    payload: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, HealthProbeError> {
    match payload.shift_remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(_) => Err(invalid(field, "it must be a string or null")),
    }
}
fn take_state(payload: &mut IndexMap<String, Json>) -> Result<DecisionsJson, HealthProbeError> {
    match payload.shift_remove("state") {
        Some(Json::Text(text)) => Ok(DecisionsJson::Text(text)),
        Some(Json::Object(entries)) => Ok(DecisionsJson::Object(entries)),
        Some(Json::List(items)) => Ok(DecisionsJson::List(items)),
        Some(Json::Null) | Some(Json::Bool(_)) | Some(Json::Number(_)) => {
            Err(invalid("state", "it must be a string, an object or a list"))
        }
        None => Err(invalid("state", "it is required")),
    }
}
fn take_questions(
    payload: &mut IndexMap<String, Json>,
) -> Result<DecisionQuestionMap, HealthProbeError> {
    let Some(Json::Object(entries)) = payload.shift_remove("questions") else {
        return Err(invalid("questions", "it must be a mapping"));
    };
    let source = serde_json::to_string(&Json::Object(entries))
        .map_err(|error| invalid_detail("questions", error.to_string()))?;
    serde_json::from_str(&source).map_err(|error| invalid_detail("questions", error.to_string()))
}
fn take_timeout(payload: &mut IndexMap<String, Json>) -> Result<Option<f64>, HealthProbeError> {
    match payload.shift_remove("timeout") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_f64()
            .map(Some)
            .ok_or_else(|| invalid("timeout", "it must fit in a number")),
        Some(_) => Err(invalid("timeout", "it must be a number or null")),
    }
}
fn take_headers(
    payload: &mut IndexMap<String, Json>,
) -> Result<Option<IndexMap<String, String>>, HealthProbeError> {
    match payload.shift_remove("extra_headers") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(entries)) => {
            let mut headers = IndexMap::new();
            for (name, value) in entries {
                let Json::Text(text) = value else {
                    return Err(invalid(
                        "extra_headers",
                        "every header value must be a string",
                    ));
                };
                headers.insert(name, text);
            }
            Ok(Some(headers))
        }
        Some(_) => Err(invalid("extra_headers", "it must be a mapping or null")),
    }
}
fn invalid(field: &'static str, detail: &str) -> HealthProbeError {
    let detail = detail.to_owned();
    HealthProbeError::EvaluationPayload { field, detail }
}
fn invalid_detail(field: &'static str, detail: String) -> HealthProbeError {
    HealthProbeError::EvaluationPayload { field, detail }
}
