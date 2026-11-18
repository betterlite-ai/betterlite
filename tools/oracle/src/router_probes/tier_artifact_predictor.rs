use betterlite_config::sections::tier_artifact_predictor as section;
use betterlite_config::sections::tier_artifact_predictor::artifact::Artifact;
use betterlite_models::decisions::Json;
use betterlite_router::adaptive_classification::request_type::RequestType;
use betterlite_router::complexity_scoring::decision::RoutingDecisionCause;
use betterlite_router::tier_artifact_predictor::artifact::{parse_artifact, resolve_tier_artifact};
use betterlite_router::tier_artifact_predictor::cohort::CohortMatcher;
use betterlite_router::tier_artifact_predictor::predictor::{
    TierArtifactPredictor, TierSuccessPredictor,
};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const MAX_PADDED_BYTES: u64 = 0x0020_0000;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    function: Function,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifact_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifact: Option<Json>,
    // a threshold the case spells as text, so the exact double survives the case file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    threshold: Option<String>,
    #[serde(default)]
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_type: Option<RequestType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifact_bytes: Option<Bytes>,
}

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

// every field is always on the wire, so a missing value compares as null rather than as absence
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    cohort: Option<String>,
    status: Option<String>,
    tier: Option<String>,
    score: Option<String>,
    signals: Option<Vec<String>>,
    cause: Option<RoutingDecisionCause>,
    probabilities: Option<Vec<String>>,
    threshold: Option<String>,
    predicted_tier: Option<String>,
    request_type: Option<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let fields = match payload.function {
        Function::SimilarityCohort => cohort_fields(payload)?,
        Function::Predict => predict_fields(payload)?,
        Function::Classify => classify_fields(payload)?,
        Function::Startup => startup_fields(payload)?,
        Function::ParseArtifact => parse_fields(payload)?,
    };
    Ok(ProbeOutput::TierArtifactPredictor(Output(Box::new(fields))))
}

fn invalid(message: &str) -> OperationError {
    OperationError::InvalidRequest {
        message: message.to_owned(),
    }
}

// an inline artifact is validated by the router; a name resolves to the packaged artifact
fn artifact_source(payload: &Payload) -> Result<section::ArtifactSource, OperationError> {
    if let Some(artifact) = &payload.artifact {
        let value =
            serde_json::to_value(artifact).map_err(|source| invalid(&source.to_string()))?;
        let parsed = serde_json::from_value::<Artifact>(value)
            .map_err(|source| invalid(&source.to_string()))?;
        return Ok(section::ArtifactSource::Inline(Box::new(parsed)));
    }
    Ok(payload
        .artifact_name
        .as_ref()
        .map_or_else(section::ArtifactSource::default, |name| {
            section::ArtifactSource::BuiltIn(name.clone())
        }))
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

fn texts(probabilities: &[f64; 4]) -> Vec<String> {
    probabilities
        .iter()
        .map(|probability| format!("{probability:.6}"))
        .collect()
}

fn cohort_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let matcher = CohortMatcher::new().map_err(|source| invalid(&source.to_string()))?;
    let cohort = matcher.similarity_cohort(&payload.prompt, request_type(payload)?);
    Ok(Fields {
        cohort: Some(cohort),
        ..Fields::default()
    })
}

fn predict_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let predictor = TierSuccessPredictor::new(
        resolve_tier_artifact(&artifact_source(payload)?)
            .map_err(|source| invalid(&source.to_string()))?,
        threshold(payload.threshold.as_deref())?,
    )
    .map_err(|source| invalid(&source.to_string()))?;
    let prediction = predictor.predict(&payload.prompt, request_type(payload)?);
    Ok(Fields {
        probabilities: Some(texts(&prediction.probabilities)),
        threshold: Some(format!("{:.6}", predictor.threshold)),
        predicted_tier: Some(prediction.required_tier.built_in().name().to_owned()),
        ..Fields::default()
    })
}

fn classify_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let predictor = TierArtifactPredictor::new(
        resolve_tier_artifact(&artifact_source(payload)?)
            .map_err(|source| invalid(&source.to_string()))?,
        threshold(payload.threshold.as_deref())?,
    )
    .map_err(|source| invalid(&source.to_string()))?;
    let outcome = predictor.classify(&payload.prompt);
    Ok(Fields {
        tier: Some(outcome.tier.name().to_owned()),
        signals: Some(outcome.signals),
        cause: Some(outcome.cause),
        probabilities: Some(texts(&outcome.forecast.probabilities)),
        threshold: Some(format!("{:.6}", outcome.forecast.threshold)),
        predicted_tier: Some(outcome.forecast.predicted_tier.name().to_owned()),
        request_type: Some(outcome.forecast.request_type.to_string()),
        ..Fields::default()
    })
}

// the reference builds the whole router here, so an invalid threshold, name or artifact rejects startup
fn startup_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let config = section::Config {
        artifact: artifact_source(payload)?,
        success_threshold: threshold(payload.threshold.as_deref())?,
    };
    let artifact = resolve_tier_artifact(&config.artifact)
        .and_then(|artifact| TierArtifactPredictor::new(artifact, config.success_threshold));
    let accepted = config.validate("heuristic_v2_success_threshold").is_ok() && artifact.is_ok();
    Ok(Fields {
        status: Some(if accepted { "accepted" } else { "rejected" }.to_owned()),
        ..Fields::default()
    })
}

fn parse_fields(payload: &Payload) -> Result<Fields, OperationError> {
    let bytes = artifact_bytes(payload)?;
    Ok(match parse_artifact(&bytes) {
        Ok(artifact) => Fields {
            status: Some("accepted".to_owned()),
            threshold: Some(format!("{:.6}", artifact.routing_threshold)),
            ..Fields::default()
        },
        Err(_error) => Fields {
            status: Some("rejected".to_owned()),
            ..Fields::default()
        },
    })
}

// the request message is capped at 1 MiB, so a case spells a large artifact as content plus a repeated pad
fn artifact_bytes(payload: &Payload) -> Result<Vec<u8>, OperationError> {
    let Some(bytes) = &payload.artifact_bytes else {
        return Err(invalid("artifact_bytes is required for parse_artifact"));
    };
    match bytes {
        Bytes::Text { content } => Ok(content.as_bytes().to_vec()),
        Bytes::Padded {
            content,
            pad,
            pad_len,
        } => {
            if pad.is_empty() || !pad.is_ascii() || *pad_len > MAX_PADDED_BYTES {
                return Err(invalid(
                    "a padded artifact needs an ASCII pad and at most 2097152 pad bytes",
                ));
            }
            let count = usize::try_from(*pad_len).map_err(|_error| {
                invalid(&format!("pad_len {pad_len} does not fit this platform"))
            })?;
            let mut builder = content.clone();
            builder.push_str(&pad.repeat(count));
            Ok(builder.into_bytes())
        }
    }
}
