use betterlite_config::sections::tier_artifact_predictor as section;
use betterlite_config::sections::tier_artifact_predictor::artifact::Artifact;
use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::complexity_scoring::decision::RoutingDecisionCause;
use betterlite_router::tier_artifact_predictor::artifact::{parse_artifact, resolve_tier_artifact};
use betterlite_router::tier_artifact_predictor::cohort::CohortMatcher;
use betterlite_router::tier_artifact_predictor::predictor::{
    TierArtifactPredictor, TierSuccessPredictor,
};
use serde::{Deserialize, Serialize};
use serde_json::Number;

mod tests;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const MAX_EXPANDED_BYTES: usize = 0x0020_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    function: Function,
    artifact_name: Option<String>,
    artifact: Option<ArtifactInput>,
    threshold: Option<String>,
    #[serde(default)]
    prompt: String,
    request_type: Option<RequestType>,
    artifact_bytes: Option<Bytes>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct ArtifactInput(Artifact);

impl Eq for ArtifactInput {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    SimilarityCohort,
    Predict,
    Classify,
    Startup,
    ParseArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Bytes {
    Text {
        content: String,
    },
    Padded {
        content: String,
        pad: String,
        pad_len: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    cohort: Option<String>,
    status: Option<String>,
    tier: Option<String>,
    score: Option<Number>,
    signals: Option<Vec<String>>,
    cause: Option<RoutingDecisionCause>,
    probabilities: Option<Vec<Number>>,
    threshold: Option<Number>,
    predicted_tier: Option<String>,
    request_type: Option<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let mut fields = Fields {
        cohort: None,
        status: None,
        tier: None,
        score: None,
        signals: None,
        cause: None,
        probabilities: None,
        threshold: None,
        predicted_tier: None,
        request_type: None,
    };
    match payload.function {
        Function::SimilarityCohort => cohort_fields(payload, &mut fields)?,
        Function::Predict => predict_fields(payload, &mut fields)?,
        Function::Classify => classify_fields(payload, &mut fields)?,
        Function::Startup => startup_fields(payload, &mut fields)?,
        Function::ParseArtifact => parse_fields(payload, &mut fields)?,
    }
    Ok(ProbeOutput::TierArtifactPredictor(Output(Box::new(fields))))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

fn artifact_source(payload: &Payload) -> section::ArtifactSource {
    match (&payload.artifact, &payload.artifact_name) {
        (Some(ArtifactInput(artifact)), _) => {
            section::ArtifactSource::Inline(Box::new(artifact.clone()))
        }
        (None, Some(name)) => section::ArtifactSource::BuiltIn(name.clone()),
        (None, None) => section::ArtifactSource::default(),
    }
}

fn threshold(text: Option<&str>) -> Result<Option<f64>, OperationError> {
    text.map(|value| {
        value
            .parse::<f64>()
            .map_err(|_error| invalid(&format!("threshold is not a number: {value}")))
    })
    .transpose()
}

fn request_type(payload: &Payload) -> Result<RequestType, OperationError> {
    payload
        .request_type
        .ok_or_else(|| invalid("request_type is required for this function"))
}

fn predictor(payload: &Payload) -> Result<TierSuccessPredictor, OperationError> {
    let artifact = resolve_tier_artifact(&artifact_source(payload))
        .map_err(|source| invalid(&source.to_string()))?;
    TierSuccessPredictor::new(artifact, threshold(payload.threshold.as_deref())?)
        .map_err(|source| invalid(&source.to_string()))
}

fn cohort_fields(payload: &Payload, fields: &mut Fields) -> Result<(), OperationError> {
    let matcher = CohortMatcher::new().map_err(|source| invalid(&source.to_string()))?;
    fields.cohort = Some(matcher.similarity_cohort(&payload.prompt, request_type(payload)?));
    Ok(())
}

fn predict_fields(payload: &Payload, fields: &mut Fields) -> Result<(), OperationError> {
    let predictor = predictor(payload)?;
    let prediction = predictor.predict(&payload.prompt, request_type(payload)?);
    fields.probabilities = Some(
        prediction
            .probabilities
            .into_iter()
            .map(number)
            .collect::<Result<_, _>>()?,
    );
    fields.threshold = Some(number(predictor.threshold)?);
    fields.predicted_tier = Some(prediction.required_tier.built_in().name().to_owned());
    Ok(())
}

fn classify_fields(payload: &Payload, fields: &mut Fields) -> Result<(), OperationError> {
    let predictor = TierArtifactPredictor::new(predictor(payload)?)
        .map_err(|source| invalid(&source.to_string()))?;
    let outcome = predictor.classify(&payload.prompt);
    fields.tier = Some(outcome.tier.name().to_owned());
    fields.score = outcome.score.map(number).transpose()?;
    fields.signals = Some(outcome.signals);
    fields.cause = Some(outcome.cause);
    fields.probabilities = Some(
        outcome
            .forecast
            .probabilities
            .into_iter()
            .map(number)
            .collect::<Result<_, _>>()?,
    );
    fields.threshold = Some(number(outcome.forecast.threshold)?);
    fields.predicted_tier = Some(outcome.forecast.predicted_tier.name().to_owned());
    fields.request_type = Some(outcome.forecast.request_type.to_string());
    Ok(())
}

fn startup_fields(payload: &Payload, fields: &mut Fields) -> Result<(), OperationError> {
    let config = section::Config {
        artifact: artifact_source(payload),
        success_threshold: threshold(payload.threshold.as_deref())?,
    };
    fields.status = Some(status_text(startup_accepted(&config)).to_owned());
    Ok(())
}

fn startup_accepted(config: &section::Config) -> bool {
    config.validate("heuristic_v2_success_threshold").is_ok()
        && resolve_tier_artifact(&config.artifact)
            .and_then(|artifact| TierSuccessPredictor::new(artifact, config.success_threshold))
            .and_then(TierArtifactPredictor::new)
            .is_ok()
}

fn parse_fields(payload: &Payload, fields: &mut Fields) -> Result<(), OperationError> {
    let artifact = parse_artifact(&artifact_bytes(payload)?);
    fields.status = Some(status_text(artifact.is_ok()).to_owned());
    fields.threshold = artifact
        .ok()
        .map(|artifact| number(artifact.routing_threshold))
        .transpose()?;
    Ok(())
}

fn status_text(accepted: bool) -> &'static str {
    if accepted { "accepted" } else { "rejected" }
}

fn artifact_bytes(payload: &Payload) -> Result<Vec<u8>, OperationError> {
    match &payload.artifact_bytes {
        None => Err(invalid("artifact_bytes is required for parse_artifact")),
        Some(Bytes::Text { content }) => expanded(content, "", 0),
        Some(Bytes::Padded {
            content,
            pad,
            pad_len,
        }) => expanded(content, pad, *pad_len),
    }
}

fn expanded(content: &str, pad: &str, pad_len: u64) -> Result<Vec<u8>, OperationError> {
    let repetitions = usize::try_from(pad_len)
        .map_err(|_error| invalid(&format!("pad_len {pad_len} does not fit this platform")))?;
    let total = pad
        .len()
        .checked_mul(repetitions)
        .and_then(|extra| content.len().checked_add(extra));
    let Some(total) = total else {
        return Err(invalid("the padded artifact overflows this platform"));
    };
    if total > MAX_EXPANDED_BYTES {
        return Err(invalid(&format!(
            "artifact over {MAX_EXPANDED_BYTES} bytes"
        )));
    }
    let mut builder = content.to_owned();
    if !pad.is_empty() {
        builder.push_str(&pad.repeat(repetitions));
    }
    Ok(builder.into_bytes())
}

fn number(value: f64) -> Result<Number, OperationError> {
    Number::from_f64(value).ok_or_else(|| OperationError::InvalidRequest {
        message: "the forecast contains a non-finite number".to_owned(),
    })
}
