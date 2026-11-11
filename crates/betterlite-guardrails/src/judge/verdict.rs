use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call::JudgeVerdict;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationInformation {
    pub eval_name: String,
    pub overall_score: f64,
    pub passed: bool,
    pub judge_model: String,
    pub threshold: f64,
    pub verdicts: Json,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EvalInformation {
    Single(EvaluationInformation),
    Many(Vec<EvaluationInformation>),
}

#[must_use]
pub fn overall_score(verdict: &JudgeVerdict) -> Option<f64> {
    match verdict.get("overall_score") {
        None => Some(100.0_f64),
        Some(Json::Number(number)) => number.as_f64().map(|score| score.clamp(0.0_f64, 100.0_f64)),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => None,
    }
}

#[must_use]
pub fn verdicts(verdict: &IndexMap<String, Json>) -> Json {
    verdict
        .get("verdicts")
        .cloned()
        .unwrap_or_else(|| Json::List(Vec::new()))
}

pub fn merge(existing: &mut Option<EvalInformation>, information: EvaluationInformation) {
    match existing.take() {
        None => {
            *existing = Some(EvalInformation::Single(information));
        }
        Some(EvalInformation::Many(mut entries)) => {
            entries.push(information);
            *existing = Some(EvalInformation::Many(entries));
        }
        Some(EvalInformation::Single(previous)) => {
            *existing = Some(EvalInformation::Many(vec![previous, information]));
        }
    }
}
