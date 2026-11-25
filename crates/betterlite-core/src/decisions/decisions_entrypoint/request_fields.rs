use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::decisions::call::fields::{
    OpenAiDecisionRequestFields, RequestFields, SystemOneRequestFields,
};
use indexmap::IndexMap;
use thiserror::Error;

mod tests;

const STATE_KEY: &str = "state";

const DEPRECATION_MESSAGE: &str = "litellm.decisions(state=...) is deprecated, call litellm.systemone for the System One request format";

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum FieldShapeError {
    #[error("questions must be a list of OpenAI decision questions")]
    QuestionsNotAList,
    #[error("questions must be a map of System One decision questions")]
    QuestionsNotAMap,
    #[error("state must be a string, a map or a list")]
    StateNotDecisionJson,
}

// _request_fields: a state keyword selects the System One envelope and warns, everything else is OpenAI's
pub(crate) fn request_fields(
    decision_input: Option<Json>,
    questions: Option<Json>,
    kwargs: &Extra,
) -> Result<RequestFields, FieldShapeError> {
    let Some(state) = kwargs.get(STATE_KEY) else {
        return Ok(RequestFields::OpenAi(OpenAiDecisionRequestFields {
            input: decision_input,
            questions: openai_questions(questions)?,
        }));
    };
    tracing::warn!(category = "DeprecationWarning", "{DEPRECATION_MESSAGE}");
    Ok(RequestFields::SystemOne(SystemOneRequestFields {
        state: state_value(state)?,
        questions: systemone_questions(questions)?,
    }))
}

fn state_value(state: &Json) -> Result<Option<Json>, FieldShapeError> {
    match state {
        Json::Null => Ok(None),
        Json::Text(_) | Json::Object(_) | Json::List(_) => Ok(Some(state.clone())),
        Json::Bool(_) | Json::Number(_) => Err(FieldShapeError::StateNotDecisionJson),
    }
}

fn openai_questions(questions: Option<Json>) -> Result<Option<Vec<Json>>, FieldShapeError> {
    match questions {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => Ok(Some(items)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(FieldShapeError::QuestionsNotAList)
        }
    }
}

pub(crate) fn systemone_questions(
    questions: Option<Json>,
) -> Result<Option<IndexMap<String, Json>>, FieldShapeError> {
    match questions {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(map)) => Ok(Some(map)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(FieldShapeError::QuestionsNotAMap)
        }
    }
}
