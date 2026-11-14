use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::support::{InfoLimits, Support, facts as empty_facts, ids, spec, with_param};
use crate::deployment_catalog::types::Candidates;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_predicates::params::{ParamSupport, ResolvedProvider};
use crate::static_eligibility::environment::{EnvironmentError, active_in_environment};
use crate::static_eligibility::error::EligibilityError;
use crate::static_eligibility::filter::{
    Cause, EligibilityInput, EligibilityResult, EligibilityServices, eligible,
};
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::web_search::WebSearchError;

const MODEL: &str = "gpt";

fn run(
    candidates: Candidates,
    facts: &RequestFacts,
    enable: bool,
    support: &Support,
) -> Result<EligibilityResult, EligibilityError> {
    let limits = InfoLimits;
    eligible(
        EligibilityInput {
            model: MODEL,
            candidates,
            facts,
            environment: Some("production"),
            enable_pre_call_checks: enable,
        },
        &EligibilityServices {
            limits: &limits,
            params: support,
        },
    )
}

fn supports() -> Support {
    Support {
        resolves: true,
        supported: Some(vec!["response_format".to_owned()]),
    }
}

fn windowed(id: &str, maximum: u64) -> DeploymentSpec {
    let info = format!(r#"{{"max_input_tokens":{maximum}}}"#);
    spec("gpt", id, "openai/gpt-4o", &info)
}

fn counted(tokens: usize) -> RequestFacts {
    let mut facts = empty_facts();
    facts.messages = Some(vec![Json::Object(JsonObject::new())]);
    facts.input_token_count = Some(tokens);
    facts
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

#[derive(Clone, Copy, Debug)]
enum Boundary {
    ContextAtTheLimitIsKept,
    ContextOneOverIsDropped,
    ContextZeroLimitNeedsAZeroCount,
    ContextWithoutACountSkips,
    OrderTargetWithoutAMatchEmpties,
    OrderNegativeIsSmallest,
    OrderModelInfoWhenParamsHasNone,
    OrderParamsWinsOverModelInfo,
    EnvironmentNotAList,
    EnvironmentUnset,
    EnvironmentUnknownValue,
    EnvironmentUnknownSupportedEntry,
    EnvironmentEmptyListIsInactive,
    EmptyPoolHasNoCauses,
    WebSearchNonBooleanFlag,
    ParamsBaseModelFromModelInfo,
    ParamsBaseModelFromParams,
    ParamsWithoutASupportedList,
    ChecksOffSkipContextAndParams,
    ContextCauseNamesTheDeploymentModel,
    WebSearchNullFlagIsSupported,
}

#[rstest]
#[case::context_at_the_limit(Boundary::ContextAtTheLimitIsKept)]
#[case::context_one_over(Boundary::ContextOneOverIsDropped)]
#[case::context_zero_limit(Boundary::ContextZeroLimitNeedsAZeroCount)]
#[case::context_without_a_count(Boundary::ContextWithoutACountSkips)]
#[case::order_target_without_a_match(Boundary::OrderTargetWithoutAMatchEmpties)]
#[case::order_negative(Boundary::OrderNegativeIsSmallest)]
#[case::order_model_info(Boundary::OrderModelInfoWhenParamsHasNone)]
#[case::order_params_wins(Boundary::OrderParamsWinsOverModelInfo)]
#[case::environment_not_a_list(Boundary::EnvironmentNotAList)]
#[case::environment_unset(Boundary::EnvironmentUnset)]
#[case::environment_unknown_value(Boundary::EnvironmentUnknownValue)]
#[case::environment_unknown_entry(Boundary::EnvironmentUnknownSupportedEntry)]
#[case::environment_empty_list(Boundary::EnvironmentEmptyListIsInactive)]
#[case::empty_pool(Boundary::EmptyPoolHasNoCauses)]
#[case::web_search_non_boolean(Boundary::WebSearchNonBooleanFlag)]
#[case::params_model_info_base_model(Boundary::ParamsBaseModelFromModelInfo)]
#[case::params_params_base_model(Boundary::ParamsBaseModelFromParams)]
#[case::params_no_supported_list(Boundary::ParamsWithoutASupportedList)]
#[case::checks_off(Boundary::ChecksOffSkipContextAndParams)]
#[case::context_cause_model(Boundary::ContextCauseNamesTheDeploymentModel)]
#[case::web_search_null_flag(Boundary::WebSearchNullFlagIsSupported)]
fn static_eligibility_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::ContextAtTheLimitIsKept => context_at_the_limit_is_kept(),
        Boundary::ContextOneOverIsDropped => context_one_over_is_dropped(),
        Boundary::ContextZeroLimitNeedsAZeroCount => context_zero_limit_needs_a_zero_count(),
        Boundary::ContextWithoutACountSkips => context_without_a_count_skips(),
        Boundary::OrderTargetWithoutAMatchEmpties => order_target_without_a_match_empties(),
        Boundary::OrderNegativeIsSmallest => order_negative_is_smallest(),
        Boundary::OrderModelInfoWhenParamsHasNone => order_source(
            vec![
                spec("gpt", "params", "openai/gpt-4o", ""),
                spec("gpt", "info", "openai/gpt-4o", r#"{"order":1}"#),
            ],
            &["info"],
        ),
        Boundary::OrderParamsWinsOverModelInfo => order_source(
            vec![
                with_param(
                    spec("gpt", "both", "openai/gpt-4o", r#"{"order":5}"#),
                    "order",
                    number(1),
                ),
                with_param(spec("gpt", "low", "openai/gpt-4o", ""), "order", number(3)),
            ],
            &["both"],
        ),
        Boundary::EnvironmentNotAList => environment_not_a_list(),
        Boundary::EnvironmentUnset => environment_unset(),
        Boundary::EnvironmentUnknownValue => environment_unknown_value(),
        Boundary::EnvironmentUnknownSupportedEntry => environment_unknown_supported_entry(),
        Boundary::EnvironmentEmptyListIsInactive => environment_empty_list_is_inactive(),
        Boundary::EmptyPoolHasNoCauses => empty_pool_has_no_causes(),
        Boundary::WebSearchNonBooleanFlag => web_search_non_boolean_flag(),
        Boundary::ParamsBaseModelFromModelInfo => params_base_model(spec(
            "gpt",
            "azure",
            "openai/gpt-4o",
            r#"{"base_model":"gpt-4"}"#,
        )),
        Boundary::ParamsBaseModelFromParams => params_base_model(with_param(
            spec("gpt", "azure", "openai/gpt-4o", ""),
            "base_model",
            Json::Text("gpt-4".to_owned()),
        )),
        Boundary::ParamsWithoutASupportedList => params_without_a_supported_list(),
        Boundary::ChecksOffSkipContextAndParams => checks_off_skip_context_and_params(),
        Boundary::ContextCauseNamesTheDeploymentModel => context_cause_names_the_deployment_model(),
        Boundary::WebSearchNullFlagIsSupported => web_search_null_flag_is_supported(),
    }
}

fn context_at_the_limit_is_kept() {
    let result = run(
        Candidates::Pool(vec![windowed("edge", 9)]),
        &counted(9),
        true,
        &supports(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["edge"]);
    assert!(result.causes.is_empty());
}

fn context_one_over_is_dropped() {
    let result = run(
        Candidates::Pool(vec![windowed("edge", 8)]),
        &counted(9),
        true,
        &supports(),
    );
    match result {
        Err(EligibilityError::ContextWindow(window)) => {
            assert!(window.causes.contains("Max Input Tokens=8"));
            assert!(window.causes.contains("Got=9"));
        }
        other => panic!("expected a context window error, got {other:?}"),
    }
}

fn context_zero_limit_needs_a_zero_count() {
    let kept = run(
        Candidates::Pool(vec![windowed("zero", 0)]),
        &counted(0),
        true,
        &supports(),
    )
    .unwrap();
    assert_eq!(ids(&kept.deployments), ["zero"]);
    let dropped = run(
        Candidates::Pool(vec![windowed("zero", 0)]),
        &counted(1),
        true,
        &supports(),
    );
    assert!(matches!(
        dropped,
        Err(EligibilityError::ContextWindow(_error))
    ));
}

fn context_without_a_count_skips() {
    let mut facts = empty_facts();
    facts.messages = Some(vec![Json::Object(JsonObject::new())]);
    let result = run(
        Candidates::Pool(vec![windowed("tiny", 1)]),
        &facts,
        true,
        &supports(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["tiny"]);
}

fn order_target_without_a_match_empties() {
    let mut facts = empty_facts();
    facts.target_order = Some(7);
    let result = run(
        Candidates::Pool(vec![
            with_param(spec("gpt", "one", "openai/gpt-4o", ""), "order", number(1)),
            with_param(spec("gpt", "two", "openai/gpt-4o", ""), "order", number(2)),
        ]),
        &facts,
        false,
        &supports(),
    )
    .unwrap();
    assert!(result.deployments.is_empty());
    assert!(result.causes.is_empty());
}

fn order_negative_is_smallest() {
    let result = run(
        Candidates::Pool(vec![
            with_param(spec("gpt", "zero", "openai/gpt-4o", ""), "order", number(0)),
            with_param(
                spec("gpt", "negative", "openai/gpt-4o", ""),
                "order",
                number(-1),
            ),
        ]),
        &RequestFacts::default(),
        false,
        &supports(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["negative"]);
}

fn order_source(deployments: Vec<DeploymentSpec>, expected: &[&str]) {
    let result = run(
        Candidates::Pool(deployments),
        &RequestFacts::default(),
        false,
        &supports(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), expected);
}

fn environment_not_a_list() {
    let info: JsonObject =
        serde_json::from_str(r#"{"supported_environments":"production"}"#).unwrap();
    let error = active_in_environment(&info, Some("production")).unwrap_err();
    match error {
        EnvironmentError::NotList { value, model_info } => {
            assert_eq!(value, "\"production\"");
            assert!(model_info.contains("supported_environments"));
        }
        other => panic!("expected a not-a-list error, got {other:?}"),
    }
}

fn environment_unset() {
    let info: JsonObject =
        serde_json::from_str(r#"{"supported_environments":["production"]}"#).unwrap();
    assert_eq!(
        active_in_environment(&info, None),
        Err(EnvironmentError::MissingEnvironment)
    );
}

fn environment_unknown_value() {
    let info: JsonObject =
        serde_json::from_str(r#"{"supported_environments":["production"]}"#).unwrap();
    assert_eq!(
        active_in_environment(&info, Some("qa")),
        Err(EnvironmentError::UnknownEnvironment {
            value: "qa".to_owned()
        })
    );
}

fn environment_unknown_supported_entry() {
    let info: JsonObject = serde_json::from_str(r#"{"supported_environments":["qa"]}"#).unwrap();
    let error = active_in_environment(&info, Some("production")).unwrap_err();
    assert!(matches!(
        error,
        EnvironmentError::UnknownSupportedEnvironment { value, .. } if value == "\"qa\""
    ));
}

fn environment_empty_list_is_inactive() {
    let result = run(
        Candidates::Pool(vec![spec(
            "gpt",
            "d1",
            "openai/gpt-4o",
            r#"{"supported_environments":[]}"#,
        )]),
        &RequestFacts::default(),
        false,
        &supports(),
    )
    .unwrap();
    assert!(result.deployments.is_empty());
    assert_eq!(
        result.causes,
        [Cause::Environment {
            model_name: "gpt".to_owned()
        }]
    );
}

fn empty_pool_has_no_causes() {
    let result = run(
        Candidates::Pool(Vec::new()),
        &RequestFacts::default(),
        false,
        &supports(),
    )
    .unwrap();
    assert!(result.deployments.is_empty());
    assert!(result.causes.is_empty());
}

fn web_search_non_boolean_flag() {
    let mut facts = empty_facts();
    facts.tools = vec![serde_json::from_str(r#"{"type":"web_search"}"#).unwrap()];
    let result = run(
        Candidates::Pool(vec![spec(
            "gpt",
            "d1",
            "openai/gpt-4o",
            r#"{"supports_web_search":"yes"}"#,
        )]),
        &facts,
        false,
        &supports(),
    );
    assert!(matches!(
        result,
        Err(EligibilityError::WebSearch(WebSearchError::NotBoolean))
    ));
}

struct ModelSupport;

impl ParamSupport for ModelSupport {
    fn is_authenticating_provider(&self, _model: &str, _custom_llm_provider: Option<&str>) -> bool {
        false
    }

    fn resolve_provider(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        Some(ResolvedProvider {
            model: model.to_owned(),
            custom_llm_provider: "openai".to_owned(),
        })
    }

    fn supported_openai_params(
        &self,
        model: &str,
        _custom_llm_provider: &str,
        _base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        match model {
            "gpt-4" => Some(Vec::new()),
            _ => Some(vec!["response_format".to_owned()]),
        }
    }
}

fn response_format_facts() -> RequestFacts {
    let mut facts = empty_facts();
    facts.messages = Some(vec![Json::Object(JsonObject::new())]);
    facts.response_format = true;
    facts
}

fn params_base_model(deployment: DeploymentSpec) {
    let dropped = eligible(
        EligibilityInput {
            model: MODEL,
            candidates: Candidates::Pool(vec![deployment]),
            facts: &response_format_facts(),
            environment: Some("production"),
            enable_pre_call_checks: true,
        },
        &EligibilityServices {
            limits: &InfoLimits,
            params: &ModelSupport,
        },
    )
    .unwrap();
    assert!(dropped.deployments.is_empty());
}

fn checks_off_skip_context_and_params() {
    let mut facts = counted(9);
    facts.response_format = true;
    let result = run(
        Candidates::Pool(vec![windowed("tiny", 1)]),
        &facts,
        false,
        &Support {
            resolves: true,
            supported: Some(Vec::new()),
        },
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["tiny"]);
    assert!(result.causes.is_empty());
}

fn context_cause_names_the_deployment_model() {
    let rows = [
        (
            spec(
                "gpt",
                "info",
                "openai/gpt-4o",
                r#"{"max_input_tokens":1,"base_model":"from-info"}"#,
            ),
            "Model=from-info",
        ),
        (
            with_param(
                windowed("params", 1),
                "base_model",
                Json::Text("from-params".to_owned()),
            ),
            "Model=from-params",
        ),
        (
            with_param(
                spec(
                    "gpt",
                    "empty",
                    "openai/gpt-4o",
                    r#"{"max_input_tokens":1,"base_model":""}"#,
                ),
                "base_model",
                Json::Text("ignored".to_owned()),
            ),
            "Model=openai/gpt-4o",
        ),
        (windowed("plain", 1), "Model=openai/gpt-4o"),
        (
            with_param(windowed("nameless", 1), "model", Json::Text(String::new())),
            "Model=gpt,",
        ),
    ];
    for (deployment, expected) in rows {
        let result = run(
            Candidates::Pool(vec![deployment]),
            &counted(9),
            true,
            &supports(),
        );
        match result {
            Err(EligibilityError::ContextWindow(window)) => {
                assert!(
                    window.causes.starts_with(expected),
                    "{} does not start with {expected}",
                    window.causes
                );
            }
            other => panic!("expected a context window error, got {other:?}"),
        }
    }
}

fn web_search_null_flag_is_supported() {
    let mut facts = empty_facts();
    facts.tools = vec![serde_json::from_str(r#"{"type":"web_search_preview"}"#).unwrap()];
    let result = run(
        Candidates::Pool(vec![spec(
            "gpt",
            "d1",
            "openai/gpt-4o",
            r#"{"supports_web_search":null}"#,
        )]),
        &facts,
        false,
        &supports(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["d1"]);
}

fn params_without_a_supported_list() {
    let result = run(
        Candidates::Pool(vec![spec("gpt", "legacy", "openai/gpt-4", "")]),
        &response_format_facts(),
        true,
        &Support {
            resolves: true,
            supported: None,
        },
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["legacy"]);
}
