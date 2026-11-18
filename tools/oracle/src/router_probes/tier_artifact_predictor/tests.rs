#![cfg(test)]

use crate::router_probes::{ProbeOperation, ProbeOutput};

#[test]
fn fractional_artifact_survives_the_probe_envelope() {
    let source = r#"{"tier_artifact_predictor":{"function":"predict","artifact":{"global_statistics":[{"tier":1,"successes":0.5,"observations":2.5}],"domain_prior_mass":4.5,"cohort_prior_mass":2.5,"routing_threshold":0.6}}}"#;
    let decoded: ProbeOperation = serde_json::from_str(source).unwrap();
    let ProbeOperation::TierArtifactPredictor(input) = decoded else {
        panic!("the envelope must select the tier artifact probe");
    };
    let artifact = input.0.artifact.unwrap().0;
    assert_eq!(artifact.routing_threshold.to_bits(), 0.6_f64.to_bits());
    assert_eq!(
        artifact.global_statistics[0].successes.to_bits(),
        0.5_f64.to_bits()
    );
    assert_eq!(
        artifact.global_statistics[0].observations.to_bits(),
        2.5_f64.to_bits()
    );
    assert_eq!(artifact.domain_prior_mass.to_bits(), 4.5_f64.to_bits());
    assert_eq!(artifact.cohort_prior_mass.to_bits(), 2.5_f64.to_bits());
}

#[test]
fn fractional_forecast_survives_the_probe_envelope() {
    let source = r#"{"tier_artifact_predictor":{"probabilities":[0.125,0.375,0.5,0.625],"threshold":0.6,"score":0.25}}"#;
    let decoded: ProbeOutput = serde_json::from_str(source).unwrap();
    let ProbeOutput::TierArtifactPredictor(output) = decoded else {
        panic!("the envelope must select the tier artifact probe");
    };
    let fields = output.0;
    assert_eq!(
        fields.threshold.unwrap().as_f64().unwrap().to_bits(),
        0.6_f64.to_bits()
    );
    assert_eq!(
        fields.score.unwrap().as_f64().unwrap().to_bits(),
        0.25_f64.to_bits()
    );
    let probabilities = fields.probabilities.unwrap();
    assert_eq!(probabilities.len(), 4);
    assert_eq!(
        probabilities[0].as_f64().unwrap().to_bits(),
        0.125_f64.to_bits()
    );
    assert_eq!(
        probabilities[3].as_f64().unwrap().to_bits(),
        0.625_f64.to_bits()
    );
}
