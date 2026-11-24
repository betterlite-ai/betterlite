use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_router::complexity_scoring::config::TierBoundaries;

use super::support::{PRESETS_URL, Script, app, get, settings};
use crate::public_endpoints::router_presets::SCORER_DEFAULTS_PATH;
use crate::public_endpoints::router_presets::scorer::{ComplexityScorerDefaults, TokenThresholds};

#[tokio::test]
async fn serves_the_shipped_scorer_defaults() {
    let script = Script::default();
    let router = app(&script, settings(PRESETS_URL, false));
    let (status, content_type, body) = get(router, SCORER_DEFAULTS_PATH).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type, "application/json");
    let served: ComplexityScorerDefaults = serde_json::from_str(&body).unwrap();
    assert_eq!(served, expected_defaults());
    assert_eq!(script.calls(), 0);
}

fn expected_defaults() -> ComplexityScorerDefaults {
    ComplexityScorerDefaults {
        tier_boundaries: TierBoundaries {
            simple_medium: 0.15,
            medium_complex: 0.35,
            complex_reasoning: 0.60,
        },
        token_thresholds: TokenThresholds {
            simple: 15,
            complex: 400,
        },
        dimension_weights: BTreeMap::from([
            ("codePresence".to_owned(), 0.30),
            ("multiStepPatterns".to_owned(), 0.03),
            ("questionComplexity".to_owned(), 0.02),
            ("reasoningMarkers".to_owned(), 0.25),
            ("simpleIndicators".to_owned(), 0.05),
            ("technicalTerms".to_owned(), 0.25),
            ("tokenCount".to_owned(), 0.10),
        ]),
    }
}
