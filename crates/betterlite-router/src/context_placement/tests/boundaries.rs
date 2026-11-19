use std::collections::BTreeMap;

use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::context_placement::ContextPlacement;
use crate::context_placement::error::PlacementError;
use crate::context_placement::facts::{ModelLimitKey, WindowFacts, deployment_limit};
use crate::context_placement::fit::ContextFit;
use crate::context_placement::placement::{Placement, PlacementRequest};
use crate::context_placement::request::{OutOfBand, RequestFacts};
use crate::tier_definitions::model::{TierDefinition, TierSet};

use super::support::{
    ExtraModelInfo, FailingCounter, FixedCounter, RecordingCounter, catalog, config, deployment,
    message, service, tier_set, with_router,
};

fn fit(
    service: &ContextPlacement<'_>,
    set: &TierSet,
    default_model: Option<&str>,
    request: &RequestFacts<'_>,
) -> ContextFit {
    super::runtime().block_on(service.request_context_fit(set, default_model, request))
}

fn place(
    service: &ContextPlacement<'_>,
    input: &PlacementRequest<'_>,
) -> Result<Option<Placement>, PlacementError> {
    super::runtime().block_on(service.context_window_placement(input, None))
}

fn window(input: u64) -> String {
    format!(r#"{{"max_input_tokens":{input}}}"#)
}

pub(super) fn run(name: &str) {
    match name {
        "needs_exactly_the_buffered_window" => needs_exactly_the_buffered_window(),
        "one_token_over_the_buffered_window" => one_token_over_the_buffered_window(),
        "the_byte_bound_equals_the_buffered_window" => the_byte_bound_equals_the_buffered_window(),
        "the_byte_bound_is_one_over_the_buffered_window" => {
            the_byte_bound_is_one_over_the_buffered_window();
        }
        "a_buffer_of_one_keeps_the_full_window" => a_buffer_of_one_keeps_the_full_window(),
        "the_fit_accepts_the_exact_capacity" => the_fit_accepts_the_exact_capacity(),
        "the_fit_rejects_one_token_over" => the_fit_rejects_one_token_over(),
        "an_unknown_tier_outside_the_ladder_is_an_error" => {
            an_unknown_tier_outside_the_ladder_is_an_error();
        }
        "an_unknown_tier_with_a_partial_pool_is_allowed" => {
            an_unknown_tier_with_a_partial_pool_is_allowed();
        }
        "a_counter_failure_leaves_the_count_absent" => a_counter_failure_leaves_the_count_absent(),
        "escalation_stops_at_the_top_tier" => escalation_stops_at_the_top_tier(),
        "a_zero_declared_limit_is_a_limit" => a_zero_declared_limit_is_a_limit(),
        "a_negative_declared_limit_is_not_a_window" => a_negative_declared_limit_is_not_a_window(),
        "a_boolean_declared_limit_is_not_a_window" => a_boolean_declared_limit_is_not_a_window(),
        "a_float_declared_limit_is_not_a_window" => a_float_declared_limit_is_not_a_window(),
        "a_text_declared_limit_is_not_a_window" => a_text_declared_limit_is_not_a_window(),
        "an_integer_above_u64_is_not_a_window" => an_integer_above_u64_is_not_a_window(),
        "an_empty_group_has_unknown_facts" | "a_wildcard_group_name_has_unknown_facts" => {
            groups_outside_the_catalog_have_unknown_facts();
        }
        "a_zero_output_limit_keeps_the_ceiling" => a_zero_output_limit_keeps_the_ceiling(),
        "an_empty_default_model_is_not_a_candidate" => an_empty_default_model_is_not_a_candidate(),
        "a_named_default_model_joins_the_facts" => a_named_default_model_joins_the_facts(),
        "a_custom_ladder_escalates_to_a_named_tier" => {
            a_custom_ladder_escalates_to_a_named_tier();
        }
        "the_count_error_names_the_failure" => the_count_error_names_the_failure(),
        _ => panic!("unknown boundary case {name}"),
    }
}

fn needs_exactly_the_buffered_window() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 95 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, None, &request);
    assert_eq!(computed.needed, Some(95));
    assert!(computed.accepts("small"));
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input).unwrap(), None);
}

fn one_token_over_the_buffered_window() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("large", r#"{"model":"openai/gpt-4o"}"#, &window(10_000)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 96 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"]), ("MEDIUM", &["large"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, None, &request);
    assert_eq!(computed.needed, Some(96));
    assert!(!computed.accepts("small"));
    assert!(computed.accepts("large"));
    assert!(computed.accepts("absent-group"));
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap().unwrap();
    assert_eq!(placement.tier, Tier::BuiltIn(ComplexityTier::Medium));
}

// a 100-byte message against a 100-token window at the 0.95 buffer: 95 fits, 96 needs a count
fn byte_bound_fit(bytes: usize) -> (ContextFit, Vec<(usize, String)>) {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = RecordingCounter::new();
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let messages = [message(&"x".repeat(bytes))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, None, &request);
    (computed, counter.seen())
}

fn the_byte_bound_equals_the_buffered_window() {
    let (computed, seen) = byte_bound_fit(95);
    assert_eq!(computed.needed, None);
    assert!(seen.is_empty());
}

fn the_byte_bound_is_one_over_the_buffered_window() {
    let (computed, seen) = byte_bound_fit(96);
    assert_eq!(computed.needed, Some(1));
    assert_eq!(seen, vec![(1, String::new())]);
}

fn a_buffer_of_one_keeps_the_full_window() {
    let config = config(true, 1.0);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = RecordingCounter::new();
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let at_the_window = [message(&"x".repeat(100))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&at_the_window),
        out_of_band,
        compaction_pending: false,
    };
    assert_eq!(fit(&placement_service, &set, None, &request).needed, None);
    assert!(counter.seen().is_empty());
    let over_the_window = [message(&"x".repeat(101))];
    let over = RequestFacts {
        messages: Some(&over_the_window),
        out_of_band,
        compaction_pending: false,
    };
    assert_eq!(fit(&placement_service, &set, None, &over).needed, Some(1));
    assert_eq!(counter.seen(), vec![(1, String::new())]);
}

fn the_fit_accepts_the_exact_capacity() {
    let exact = ContextFit {
        facts: BTreeMap::from([(
            "small".to_owned(),
            WindowFacts {
                window: Some(100),
                has_unknown: false,
            },
        )]),
        needed: Some(95),
        buffer: 0.95,
    };
    assert!(exact.accepts("small"));
    let unknown = ContextFit {
        facts: BTreeMap::from([(
            "odd".to_owned(),
            WindowFacts {
                window: Some(101),
                has_unknown: true,
            },
        )]),
        needed: Some(95),
        buffer: 0.95,
    };
    assert!(unknown.accepts("odd"));
}

fn the_fit_rejects_one_token_over() {
    let over = ContextFit {
        facts: BTreeMap::from([(
            "small".to_owned(),
            WindowFacts {
                window: Some(100),
                has_unknown: false,
            },
        )]),
        needed: Some(96),
        buffer: 0.95,
    };
    assert!(!over.accepts("small"));
    let truncated = ContextFit {
        facts: BTreeMap::from([(
            "odd".to_owned(),
            WindowFacts {
                window: Some(101),
                has_unknown: false,
            },
        )]),
        needed: Some(96),
        buffer: 0.95,
    };
    assert!(!truncated.accepts("odd"));
    assert_eq!(truncated.needed, Some(96));
}

fn an_unknown_tier_outside_the_ladder_is_an_error() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 200 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("MISSING", &["small"])]);
    let tier = Tier::Named("MISSING".to_owned());
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let error = place(&placement_service, &input).unwrap_err();
    assert_eq!(
        error,
        PlacementError::UnknownTier {
            tier: "MISSING".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "complexity tier MISSING is not in the active tier set"
    );
}

fn an_unknown_tier_with_a_partial_pool_is_allowed() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("tiny", r#"{"model":"openai/gpt-4o-mini"}"#, &window(50)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 60 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("MISSING", &["small", "tiny"])]);
    let tier = Tier::Named("MISSING".to_owned());
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap().unwrap();
    assert_eq!(placement.tier, Tier::Named("MISSING".to_owned()));
    assert_eq!(placement.allowed_models, vec!["small".to_owned()]);
}

fn a_counter_failure_leaves_the_count_absent() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FailingCounter;
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, None, &request);
    assert_eq!(computed.needed, None);
    assert_eq!(
        computed.facts.get("small"),
        Some(&WindowFacts {
            window: Some(100),
            has_unknown: false,
        })
    );
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input).unwrap(), None);
}

fn escalation_stops_at_the_top_tier() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 200 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("COMPLEX", &["small"])]);
    let tier = Tier::BuiltIn(ComplexityTier::Complex);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    assert_eq!(place(&placement_service, &input).unwrap(), None);
}

fn a_zero_declared_limit_is_a_limit() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![with_router(
        deployment("group", r#"{"model":"openai/gpt-4o"}"#, &window(0)),
        r#"{"max_input_tokens":500}"#,
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let spec = deployment("group", r#"{"model":"openai/gpt-4o"}"#, &window(0));
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::InputTokens),
        Some(0)
    );
    let placement_service = service(&config, &deployments, &info, &counter);
    assert_eq!(
        placement_service.group_window_facts("group"),
        WindowFacts {
            window: Some(0),
            has_unknown: false,
        }
    );
}

// a non-integer declared limit shadows the router's 500-token map entry and yields no window
fn declared_limit(raw: &str) -> Option<u64> {
    let info = ExtraModelInfo;
    let shadowed = with_router(
        deployment(
            "group",
            r#"{"model":"openai/gpt-4o"}"#,
            &format!(r#"{{"max_input_tokens":{raw}}}"#),
        ),
        r#"{"max_input_tokens":500}"#,
    );
    deployment_limit(&info, "group", &shadowed, ModelLimitKey::InputTokens)
}

fn a_negative_declared_limit_is_not_a_window() {
    let limit = declared_limit("-1");
    assert_eq!(limit, None);
}

fn a_boolean_declared_limit_is_not_a_window() {
    let limit = declared_limit("true");
    assert_eq!(limit, None);
}

fn a_float_declared_limit_is_not_a_window() {
    let limit = declared_limit("1.5");
    assert_eq!(limit, None);
}

fn a_text_declared_limit_is_not_a_window() {
    let limit = declared_limit(r#""100""#);
    assert_eq!(limit, None);
}

fn an_integer_above_u64_is_not_a_window() {
    let limit = declared_limit("18446744073709551616");
    assert_eq!(limit, None);
}

// a group name that matches no deployment, wildcard or not, has no window facts to prove anything with
fn groups_outside_the_catalog_have_unknown_facts() {
    let config = config(true, 0.95);
    let wildcard = catalog(vec![deployment(
        "team-*",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let empty = catalog(Vec::new());
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let wildcard_service = service(&config, &wildcard, &info, &counter);
    assert_eq!(
        wildcard_service.group_window_facts("team-gpt"),
        WindowFacts::UNKNOWN
    );
    let empty_service = service(&config, &empty, &info, &counter);
    assert_eq!(
        empty_service.group_window_facts("nothing"),
        WindowFacts::UNKNOWN
    );
    assert_eq!(empty_service.group_output_ceiling("nothing"), None);
}

fn a_zero_output_limit_keeps_the_ceiling() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment(
            "group",
            r#"{"model":"openai/gpt-4o"}"#,
            r#"{"max_output_tokens":0}"#,
        ),
        deployment(
            "group",
            r#"{"model":"openai/gpt-4o"}"#,
            r#"{"max_output_tokens":50}"#,
        ),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    assert_eq!(placement_service.group_output_ceiling("group"), Some(0));
}

fn an_empty_default_model_is_not_a_candidate() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let messages = [message(&"x".repeat(40))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, Some(""), &request);
    let names: Vec<&str> = computed.facts.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["small"]);
}

fn a_named_default_model_joins_the_facts() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![deployment(
        "small",
        r#"{"model":"openai/gpt-4o"}"#,
        &window(100),
    )]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 1 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let set = tier_set(&[("SIMPLE", &["small"])]);
    let messages = [message(&"x".repeat(40))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let computed = fit(&placement_service, &set, Some("fallback"), &request);
    assert_eq!(computed.facts.get("fallback"), Some(&WindowFacts::UNKNOWN));
    assert_eq!(computed.needed, None);
}

fn a_custom_ladder_escalates_to_a_named_tier() {
    let config = config(true, 0.95);
    let deployments = catalog(vec![
        deployment("small", r#"{"model":"openai/gpt-4o"}"#, &window(100)),
        deployment("large", r#"{"model":"openai/gpt-4o"}"#, &window(10_000)),
    ]);
    let info = ExtraModelInfo;
    let counter = FixedCounter { value: 200 };
    let placement_service = service(&config, &deployments, &info, &counter);
    let mut set = tier_set(&[("SIMPLE", &["small"]), ("MEDIUM", &["large"])]);
    set.definitions = Some(vec![
        TierDefinition {
            name: "SIMPLE".to_owned(),
            description: None,
        },
        TierDefinition {
            name: "MEDIUM".to_owned(),
            description: None,
        },
    ]);
    let tier = Tier::BuiltIn(ComplexityTier::Simple);
    let messages = [message(&"x".repeat(200))];
    let out_of_band = OutOfBand::default();
    let request = RequestFacts {
        messages: Some(&messages),
        out_of_band,
        compaction_pending: false,
    };
    let input = PlacementRequest {
        tier: &tier,
        set: &set,
        default_model: None,
        pool_override: None,
        request,
    };
    let placement = place(&placement_service, &input).unwrap().unwrap();
    assert_eq!(placement.tier, Tier::Named("MEDIUM".to_owned()));
    assert_eq!(placement.allowed_models, vec!["large".to_owned()]);
}

fn the_count_error_names_the_failure() {
    let error = PlacementError::Count {
        message: "the counter failed".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "the request token count failed: the counter failed"
    );
}
