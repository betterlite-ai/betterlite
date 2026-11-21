use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::cache_cost_routing as section;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{CancelledChooser, FixedChooser, Fixture, choice, fit, response};

#[rstest]
#[case::exact("0.001", "0.00100000")]
#[case::pads("0.00000001", "0.00000001")]
#[case::half_even_down("0.000000005", "0.00000000")]
#[case::half_even_up("0.000000015", "0.00000002")]
#[tokio::test]
async fn cost_text_rounds_to_eight_places(#[case] estimated: &str, #[case] written: &str) {
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(Some(choice(estimated, "0.004")), Duration::ZERO);
    let applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    let decision = applied.routing_decision.unwrap();
    let Json::List(signals) = &decision["signals"] else {
        panic!("signals is a list");
    };
    let Json::Text(last) = &signals[2] else {
        panic!("a signal is text");
    };
    assert_eq!(
        last,
        &format!("cache-aware:estimated-cost={written};original-cost=0.00400000")
    );
}

#[rstest]
#[case::inside(Duration::ZERO, true)]
#[case::expired(Duration::from_millis(200), false)]
#[tokio::test]
async fn the_prediction_deadline_bounds_a_slow_chooser(
    #[case] delay: Duration,
    #[case] stamped: bool,
) {
    let mut fixture = Fixture::new();
    fixture.config.timeout_ms = 50;
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), delay);
    let applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    assert_eq!(applied.model == "medium", stamped);
    assert_eq!(chooser.calls(), 1);
}

#[tokio::test]
async fn an_expired_deadline_drops_the_prediction() {
    let mut fixture = Fixture::new();
    fixture.config.timeout_ms = 10;
    let dropped = Arc::new(AtomicBool::new(false));
    let chooser = CancelledChooser {
        dropped: Arc::clone(&dropped),
    };
    let applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    assert_eq!(applied.model, "small");
    assert!(dropped.load(Ordering::SeqCst));
}

#[rstest]
#[case::exactly_at_the_buffered_window(Some(80_u64), true)]
#[case::just_over_the_buffered_window(Some(81_u64), false)]
#[case::no_count_at_all(None, true)]
#[tokio::test]
async fn the_context_gate_accepts_exactly_the_buffered_window(
    #[case] needed: Option<u64>,
    #[case] stamped: bool,
) {
    let mut fixture = Fixture::new();
    fixture.fit = fit(&[("medium", Some(100))], needed);
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    assert_eq!(applied.model == "medium", stamped);
}

#[tokio::test]
async fn a_zero_output_budget_reaches_the_chooser() {
    let mut fixture = Fixture::new();
    fixture.config.output_tokens = 0;
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let _applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    assert_eq!(chooser.seen().map(|seen| seen.output_tokens), Some(0));
}

fn load(fields: &str, variables: &[(&str, &str)]) -> Result<Vec<Option<section::Config>>, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let source = format!(
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n      complexity_router_config:\n        {fields}\n"
    );
    ConfigLoader { lookup }
        .load(&source)
        .map(|loaded| loaded.sections.cache_cost_routing.deployments)
        .map_err(|error| error.to_string())
}

#[test]
fn config_defaults_match_the_declared_keys() {
    let deployments = load("cache_aware_routing: true", &[]).unwrap();
    assert_eq!(
        deployments,
        vec![Some(section::Config {
            enabled: true,
            output_tokens: 1024,
            timeout_ms: 2000,
        })]
    );
}

#[test]
fn an_absent_complexity_router_config_leaves_the_deployment_unset() {
    let source =
        "model_list:\n  - model_name: router\n    litellm_params:\n      model: openai/gpt-4o\n";
    let deployments = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .cache_cost_routing
    .deployments;
    assert_eq!(deployments, vec![None]);
}

#[test]
fn a_zero_output_budget_is_allowed() {
    let deployments = load("cache_aware_routing_output_tokens: 0", &[]).unwrap();
    assert_eq!(deployments[0].unwrap().output_tokens, 0);
}

#[test]
fn a_zero_timeout_is_refused_with_its_path() {
    let error = load("cache_aware_routing_timeout_ms: 0", &[]).unwrap_err();
    assert!(
        error.contains(
            "model_list.0.litellm_params.complexity_router_config.cache_aware_routing_timeout_ms"
        ),
        "{error}"
    );
}

#[test]
fn a_negative_output_budget_is_refused() {
    let error = load("cache_aware_routing_output_tokens: -1", &[]).unwrap_err();
    assert!(
        error.contains("cache_aware_routing_output_tokens"),
        "{error}"
    );
}

#[test]
fn an_environment_flag_resolves() {
    let deployments = load("cache_aware_routing: os.environ/CACHE", &[("CACHE", "yes")]).unwrap();
    assert!(deployments[0].unwrap().enabled);
}

#[test]
fn an_empty_environment_flag_takes_the_default() {
    let deployments = load("cache_aware_routing: os.environ/CACHE", &[("CACHE", "")]).unwrap();
    assert_eq!(deployments[0].unwrap().enabled, section::DEFAULT_ENABLED);
}

#[test]
fn a_missing_environment_value_is_named() {
    let error = load("cache_aware_routing_timeout_ms: os.environ/TIMEOUT", &[]).unwrap_err();
    assert!(error.contains("TIMEOUT"), "{error}");
}
