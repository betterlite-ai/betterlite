use crate::guardrails::model_armor::streams::assembly::Assembly;
use crate::guardrails::model_armor::streams::outcome::{Outcome, resolve_outcome};

use super::support::{FakeWire, Setup, Verdict, value};

fn model() -> Assembly {
    Assembly::Model(value(r#"{"choices": []}"#))
}

#[test]
fn resolve_outcome_blocks_a_match_even_with_masking_on() {
    let wire = FakeWire::new(Setup {
        verdict: Verdict {
            block: true,
            ..Verdict::default()
        },
        ..Setup::default()
    });
    let outcome = resolve_outcome(&wire, true, "text", &model(), &value("{}"));
    assert_eq!(
        outcome,
        Outcome {
            blocked: true,
            sanitized: None,
        }
    );
}

#[test]
fn resolve_outcome_passes_masking_as_allow_sanitization() {
    let wire = FakeWire::new(Setup::default());
    let outcome = resolve_outcome(&wire, true, "text", &model(), &value("{}"));
    assert_eq!(outcome, Outcome::default());
    assert_eq!(wire.block_args(), vec![true]);
}

#[test]
fn resolve_outcome_without_masking_never_rewrites() {
    let wire = FakeWire::new(Setup {
        sanitized: Some("[REDACTED]".to_owned()),
        ..Setup::default()
    });
    let outcome = resolve_outcome(&wire, false, "secret", &model(), &value("{}"));
    assert_eq!(outcome, Outcome::default());
    assert_eq!(wire.block_args(), vec![false]);
}

#[test]
fn resolve_outcome_fails_closed_on_a_deidentify_match_without_a_rewrite() {
    let wire = FakeWire::new(Setup {
        verdict: Verdict {
            deidentify: true,
            ..Verdict::default()
        },
        ..Setup::default()
    });
    let outcome = resolve_outcome(&wire, true, "secret", &model(), &value("{}"));
    assert_eq!(
        outcome,
        Outcome {
            blocked: true,
            sanitized: None,
        }
    );
}

#[test]
fn resolve_outcome_blocks_when_the_rewrite_is_empty() {
    let wire = FakeWire::new(Setup {
        verdict: Verdict {
            deidentify: true,
            ..Verdict::default()
        },
        sanitized: Some(String::new()),
        ..Setup::default()
    });
    let outcome = resolve_outcome(&wire, true, "secret", &model(), &value("{}"));
    assert!(outcome.blocked);
    assert_eq!(outcome.sanitized, None);
}

#[test]
fn resolve_outcome_skips_a_rewrite_equal_to_the_scanned_content() {
    let wire = FakeWire::new(Setup {
        sanitized: Some("secret".to_owned()),
        ..Setup::default()
    });
    let outcome = resolve_outcome(&wire, true, "secret", &model(), &value("{}"));
    assert_eq!(outcome, Outcome::default());
}

#[test]
fn resolve_outcome_blocks_a_rewrite_it_cannot_emit_on_the_surface() {
    let wire = FakeWire::new(Setup {
        sanitized: Some("[REDACTED]".to_owned()),
        ..Setup::default()
    });
    let responses = Assembly::Responses(value("{}"));
    let outcome = resolve_outcome(&wire, true, "secret", &responses, &value("{}"));
    assert_eq!(
        outcome,
        Outcome {
            blocked: true,
            sanitized: None,
        }
    );
}

#[test]
fn resolve_outcome_returns_the_rewrite_for_a_model_response() {
    let wire = FakeWire::new(Setup {
        sanitized: Some("[REDACTED]".to_owned()),
        ..Setup::default()
    });
    let outcome = resolve_outcome(&wire, true, "secret", &model(), &value("{}"));
    assert_eq!(
        outcome,
        Outcome {
            blocked: false,
            sanitized: Some("[REDACTED]".to_owned()),
        }
    );
}

#[test]
fn resolve_outcome_blocks_a_completion_rewrite_it_cannot_emit() {
    let wire = FakeWire::new(Setup {
        sanitized: Some("[REDACTED]".to_owned()),
        ..Setup::default()
    });
    let completion = Assembly::Completion(value("{}"));
    let outcome = resolve_outcome(&wire, true, "secret", &completion, &value("{}"));
    assert!(outcome.blocked);
}
