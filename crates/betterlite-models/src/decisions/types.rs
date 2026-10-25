#![expect(
    clippy::disallowed_types,
    clippy::derive_partial_eq_without_eq,
    reason = "decision records preserve arbitrary JSON and probabilities use floating point"
)]

use std::collections::{BTreeMap, BTreeSet};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub type DecisionsJson = Value;
pub type NoulCriteria = BTreeMap<String, Option<DecisionsJson>>;
pub type ExtraFields = BTreeMap<String, Value>;

fn validate_length<T>(value: &T, min: usize, max: usize) -> Result<(), String>
where
    T: Length + ?Sized,
{
    let length = value.length();
    if (min..=max).contains(&length) {
        Ok(())
    } else {
        Err(format!("length {length} is outside {min}..={max}"))
    }
}

trait Length {
    fn length(&self) -> usize;
}
impl<T> Length for [T] {
    fn length(&self) -> usize {
        self.len()
    }
}
impl<K, V> Length for BTreeMap<K, V> {
    fn length(&self) -> usize {
        self.len()
    }
}

fn sequence<'de, D, T>(deserializer: D, min: usize, max: usize) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = Vec::<T>::deserialize(deserializer)?;
    validate_length(value.as_slice(), min, max).map_err(D::Error::custom)?;
    Ok(value)
}

fn map<'de, D>(deserializer: D) -> Result<BTreeMap<String, Option<Value>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = BTreeMap::<String, Option<Value>>::deserialize(deserializer)?;
    validate_length(&value, 1, 255).map_err(D::Error::custom)?;
    Ok(value)
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum DecisionQuestion {
    #[serde(rename = "noul")]
    Noul(NoulQuestion),
    #[serde(rename = "choice")]
    Choice(ChoiceQuestion),
    #[serde(rename = "score")]
    Score(ScoreQuestion),
}

impl<'de> Deserialize<'de> for DecisionQuestion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value.get("type").and_then(Value::as_str) {
            Some("noul") => Ok(Self::Noul(
                serde_json::from_value(value).map_err(D::Error::custom)?,
            )),
            Some("choice") => Ok(Self::Choice(
                serde_json::from_value(value).map_err(D::Error::custom)?,
            )),
            Some("score") => Ok(Self::Score(
                serde_json::from_value(value).map_err(D::Error::custom)?,
            )),
            _ => Err(D::Error::custom("unknown decision question type")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "NoulQuestionWire")]
pub struct NoulQuestion {
    pub r#type: String,
    pub instructions: Option<DecisionsJson>,
    pub criteria: Option<NoulCriteria>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoulQuestionWire {
    r#type: String,
    instructions: Option<DecisionsJson>,
    criteria: Option<NoulCriteria>,
}

impl TryFrom<NoulQuestionWire> for NoulQuestion {
    type Error = String;

    fn try_from(wire: NoulQuestionWire) -> Result<Self, Self::Error> {
        let question = Self {
            r#type: wire.r#type,
            instructions: wire.instructions,
            criteria: wire.criteria,
        };
        question.validate()?;
        Ok(question)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChoiceQuestion {
    pub r#type: String,
    pub instructions: Option<DecisionsJson>,
    #[serde(deserialize_with = "map")]
    pub criteria: BTreeMap<String, Option<DecisionsJson>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreQuestion {
    pub r#type: String,
    pub instructions: Option<DecisionsJson>,
    #[serde(deserialize_with = "score_criteria")]
    pub criteria: Vec<DecisionsJson>,
}

fn score_criteria<'de, D>(deserializer: D) -> Result<Vec<DecisionsJson>, D::Error>
where
    D: Deserializer<'de>,
{
    sequence(deserializer, 1, 10)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DecisionsRequestBody {
    pub state: DecisionsJson,
    #[serde(deserialize_with = "question_map")]
    pub questions: BTreeMap<String, DecisionQuestion>,
    #[serde(flatten)]
    pub extra: ExtraFields,
}

fn question_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, DecisionQuestion>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = BTreeMap::deserialize(deserializer)?;
    validate_length(&value, 1, 128).map_err(D::Error::custom)?;
    Ok(value)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DecisionsRequest {
    pub model: String,
    #[serde(flatten)]
    pub body: DecisionsRequestBody,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum DecisionAnswer {
    Noul(NoulAnswer),
    Choice(ChoiceAnswer),
    Score(ScoreAnswer),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NoulAnswer {
    pub r#type: String,
    pub noul: f64,
    #[serde(flatten)]
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChoiceAnswer {
    pub r#type: String,
    pub choice: String,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
    #[serde(flatten)]
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoreAnswer {
    pub r#type: String,
    pub score: f64,
    pub confidence: f64,
    pub legend: BTreeMap<String, DecisionsJson>,
    pub probabilities: BTreeMap<String, f64>,
    #[serde(flatten)]
    pub extra: ExtraFields,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DecisionsUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(skip_serializing)]
    pub cached_tokens: i64,
    #[serde(skip_serializing)]
    pub cache_write_tokens: i64,
    #[serde(flatten)]
    pub extra: ExtraFields,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DecisionsResponse {
    pub model: Option<String>,
    pub answers: BTreeMap<String, DecisionAnswer>,
    pub usage: Option<DecisionsUsage>,
    #[serde(flatten)]
    pub extra: ExtraFields,
    #[serde(skip)]
    pub hidden: ExtraFields,
}
impl DecisionsResponse {
    #[must_use]
    pub const fn hidden_params(&self) -> &ExtraFields {
        &self.hidden
    }
    pub fn set_hidden_params(&mut self, params: &ExtraFields) {
        self.hidden.extend(params.clone());
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OpenAIDecisionInput {
    Text(String),
    Messages(Vec<OpenAIDecisionInputMessage>),
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIDecisionInputText {
    pub r#type: String,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIDecisionInputImage {
    pub r#type: String,
    pub image_url: String,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OpenAIDecisionContentPart {
    Text(OpenAIDecisionInputText),
    Image(OpenAIDecisionInputImage),
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIDecisionInputMessage {
    pub role: String,
    pub r#type: String,
    pub content: OpenAIContent,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OpenAIContent {
    Text(String),
    Parts(Vec<OpenAIDecisionContentPart>),
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum OpenAIDecisionQuestion {
    Predicate(OpenAIPredicateQuestion),
    Choice(OpenAIChoiceQuestion),
    Score(OpenAIScoreQuestion),
}

impl<'de> Deserialize<'de> for OpenAIDecisionQuestion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value.get("type").and_then(Value::as_str) {
            Some("predicate") => Ok(Self::Predicate(
                serde_json::from_value(value).map_err(D::Error::custom)?,
            )),
            Some("choice") => Ok(Self::Choice(
                serde_json::from_value(value).map_err(D::Error::custom)?,
            )),
            Some("score") => Ok(Self::Score(
                serde_json::from_value(value).map_err(D::Error::custom)?,
            )),
            _ => Err(D::Error::custom("unknown OpenAI decision question type")),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIPredicateQuestion {
    pub r#type: String,
    pub name: Option<String>,
    pub instructions: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIChoiceOption {
    pub value: ChoiceValue,
    pub description: Option<String>,
}
pub type ChoiceValue = Value;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "OpenAIChoiceQuestionWire")]
pub struct OpenAIChoiceQuestion {
    pub r#type: String,
    pub name: Option<String>,
    pub instructions: String,
    #[serde(deserialize_with = "choice_options")]
    pub choices: Vec<OpenAIChoiceOption>,
}
fn choice_options<'de, D>(deserializer: D) -> Result<Vec<OpenAIChoiceOption>, D::Error>
where
    D: Deserializer<'de>,
{
    sequence(deserializer, 2, 255)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenAIChoiceQuestionWire {
    r#type: String,
    name: Option<String>,
    instructions: String,
    #[serde(deserialize_with = "choice_options")]
    choices: Vec<OpenAIChoiceOption>,
}

impl TryFrom<OpenAIChoiceQuestionWire> for OpenAIChoiceQuestion {
    type Error = String;

    fn try_from(wire: OpenAIChoiceQuestionWire) -> Result<Self, Self::Error> {
        let question = Self {
            r#type: wire.r#type,
            name: wire.name,
            instructions: wire.instructions,
            choices: wire.choices,
        };
        question.validate()?;
        Ok(question)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIScoreLevel {
    pub label: String,
    pub description: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAIScoreQuestion {
    pub r#type: String,
    pub name: Option<String>,
    pub instructions: String,
    #[serde(deserialize_with = "score_levels")]
    pub levels: Vec<OpenAIScoreLevel>,
}
fn score_levels<'de, D>(deserializer: D) -> Result<Vec<OpenAIScoreLevel>, D::Error>
where
    D: Deserializer<'de>,
{
    sequence(deserializer, 2, 10)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIDecisionRequestBody {
    pub input: OpenAIDecisionInput,
    #[serde(deserialize_with = "openai_questions")]
    pub questions: Vec<OpenAIDecisionQuestion>,
    pub safety_identifier: Option<String>,
    #[serde(flatten)]
    pub extra: ExtraFields,
}
fn openai_questions<'de, D>(deserializer: D) -> Result<Vec<OpenAIDecisionQuestion>, D::Error>
where
    D: Deserializer<'de>,
{
    sequence(deserializer, 1, 128)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIDecisionRequest {
    pub model: String,
    #[serde(flatten)]
    pub body: OpenAIDecisionRequestBody,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIPredicateAnswer {
    pub r#type: String,
    pub name: Option<String>,
    pub probability: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIChoiceProbability {
    pub value: ChoiceValue,
    pub probability: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIChoiceAnswer {
    pub r#type: String,
    pub name: Option<String>,
    pub choice: ChoiceValue,
    pub probabilities: Vec<OpenAIChoiceProbability>,
    pub confidence: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIScoreProbability {
    pub value: i64,
    pub label: DecisionsJson,
    pub probability: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIScoreAnswer {
    pub r#type: String,
    pub name: Option<String>,
    pub score: f64,
    pub probabilities: Vec<OpenAIScoreProbability>,
    pub confidence: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIRefusalAnswer {
    pub r#type: String,
    pub name: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum OpenAIDecisionAnswer {
    Predicate(OpenAIPredicateAnswer),
    Choice(OpenAIChoiceAnswer),
    Score(OpenAIScoreAnswer),
    Refusal(OpenAIRefusalAnswer),
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenAIDecisionInputTokensDetails {
    pub cached_tokens: i64,
    pub cache_write_tokens: i64,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenAIDecisionOutputTokensDetails {
    pub reasoning_tokens: i64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIDecisionUsage {
    pub input_tokens: i64,
    pub input_tokens_details: OpenAIDecisionInputTokensDetails,
    pub output_tokens: i64,
    pub output_tokens_details: OpenAIDecisionOutputTokensDetails,
    pub total_tokens: i64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenAIDecisionResponse {
    pub model: String,
    pub answers: Vec<OpenAIDecisionAnswer>,
    pub usage: OpenAIDecisionUsage,
    #[serde(flatten)]
    pub extra: ExtraFields,
    #[serde(skip)]
    pub hidden: ExtraFields,
}
impl OpenAIDecisionResponse {
    #[must_use]
    pub const fn hidden_params(&self) -> &ExtraFields {
        &self.hidden
    }
    pub fn set_hidden_params(&mut self, params: &ExtraFields) {
        self.hidden.extend(params.clone());
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DecisionsIRInput {
    State(DecisionsJson),
    Messages(Vec<OpenAIDecisionInputMessage>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRPredicateQuestion {
    pub name: Option<String>,
    pub instructions: Option<DecisionsJson>,
    pub criteria: Option<NoulCriteria>,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRChoiceOption {
    pub value: ChoiceValue,
    pub description: Option<DecisionsJson>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRChoiceQuestion {
    pub name: Option<String>,
    pub instructions: Option<DecisionsJson>,
    pub choices: Vec<DecisionsIRChoiceOption>,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRScoreLevel {
    pub label: DecisionsJson,
    pub description: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRScoreQuestion {
    pub name: Option<String>,
    pub instructions: Option<DecisionsJson>,
    pub levels: Vec<DecisionsIRScoreLevel>,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DecisionsIRQuestion {
    Predicate(DecisionsIRPredicateQuestion),
    Choice(DecisionsIRChoiceQuestion),
    Score(DecisionsIRScoreQuestion),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRRequest {
    pub input: DecisionsIRInput,
    pub questions: Vec<DecisionsIRQuestion>,
    pub safety_identifier: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRPredicateAnswer {
    pub probability: f64,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRChoiceProbability {
    pub value: ChoiceValue,
    pub probability: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRChoiceAnswer {
    pub choice: ChoiceValue,
    pub confidence: f64,
    pub probabilities: Vec<DecisionsIRChoiceProbability>,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRScoreProbability {
    pub value: i64,
    pub label: DecisionsJson,
    pub probability: f64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRScoreAnswer {
    pub score: f64,
    pub confidence: f64,
    pub probabilities: Vec<DecisionsIRScoreProbability>,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRRefusal;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DecisionsIRAnswer {
    Predicate(DecisionsIRPredicateAnswer),
    Choice(DecisionsIRChoiceAnswer),
    Score(DecisionsIRScoreAnswer),
    Refusal(DecisionsIRRefusal),
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionsIRResponse {
    pub model: Option<String>,
    pub answers: Vec<DecisionsIRAnswer>,
    pub usage: DecisionsIRUsage,
    pub extra: ExtraFields,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedDecisionsRequest {
    pub reason: String,
}

#[must_use]
pub fn systemone_choice_key(value: &ChoiceValue) -> Option<String> {
    match value {
        Value::Bool(boolean) => Some(boolean.to_string()),
        Value::String(string) => Some(string.clone()),
        Value::Null | Value::Number(_) | Value::Array(_) | Value::Object(_) => None,
    }
}

impl NoulQuestion {
    pub fn validate(&self) -> Result<(), String> {
        if self.instructions.is_none() && self.criteria.is_none() {
            Err("a noul question requires instructions or criteria".to_owned())
        } else {
            Ok(())
        }
    }
}
impl OpenAIChoiceQuestion {
    pub fn validate(&self) -> Result<(), String> {
        let mut keys = BTreeSet::new();
        for option in &self.choices {
            let key = systemone_choice_key(&option.value)
                .ok_or_else(|| "choice values must be strings or booleans".to_owned())?;
            if !keys.insert(key) {
                return Err("choice values must be unique".to_owned());
            }
        }
        Ok(())
    }
}
