use super::calls::EvaluationCall;
use super::error::HealthProbeError;
use super::params::HealthProbeParams;
use betterlite_models::decisions::systemone::DecisionQuestionMap;
use betterlite_models::decisions::{DecisionsJson, Extra, Json};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Serialize;
use serde::de::DeserializeOwned;
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
    let hosted = custom_llm_provider == HOSTED_VLLM_PROVIDER;
    let question_type = if hosted { "choice" } else { "noul" };
    let mut question = IndexMap::new();
    question.insert("type".to_owned(), Json::Text(question_type.to_owned()));
    question.insert(
        "instructions".to_owned(),
        Json::Text(HEALTH_CHECK_QUESTION.to_owned()),
    );
    if hosted {
        let mut criteria = IndexMap::new();
        criteria.insert("yes".to_owned(), Json::Null);
        criteria.insert("no".to_owned(), Json::Null);
        question.insert("criteria".to_owned(), Json::Object(criteria));
    }
    let mut questions = IndexMap::new();
    questions.insert(QUESTION_NAME.to_owned(), Json::Object(question));
    questions
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
    for (key, value) in params.call_parameters(false)? {
        payload.insert(key, value);
    }
    let mut validated = validate_payload(payload)?;
    validated.api_key.clone_from(&params.api_key);
    Ok(EvaluationCall {
        params: validated,
        logging: params.litellm_logging_obj.clone(),
    })
}
fn take_field<F>(
    payload: &mut IndexMap<String, Json>,
    key: &'static str,
) -> Result<F, HealthProbeError>
where
    F: DeserializeOwned,
{
    let value = payload.shift_remove(key).unwrap_or(Json::Null);
    let text = serde_json::to_string(&value)
        .map_err(|source| HealthProbeError::EvaluationPayload { field: key, source })?;
    serde_json::from_str(&text)
        .map_err(|source| HealthProbeError::EvaluationPayload { field: key, source })
}
fn validate_payload(
    mut payload: IndexMap<String, Json>,
) -> Result<EvaluationCallParams, HealthProbeError> {
    Ok(EvaluationCallParams {
        model: take_field(&mut payload, "model")?,
        state: take_field(&mut payload, "state")?,
        questions: take_field(&mut payload, "questions")?,
        api_base: take_field(&mut payload, "api_base")?,
        custom_llm_provider: take_field(&mut payload, "custom_llm_provider")?,
        timeout: take_field(&mut payload, "timeout")?,
        extra_headers: take_field(&mut payload, "extra_headers")?,
        extra: payload,
        api_key: None,
    })
}
