use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    Check, Cooldown, InfoLimits, Support, facts as empty_facts, ids, message, spec, with_param,
};
use crate::deployment_catalog::types::Candidates;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::static_eligibility::error::EligibilityError;
use crate::static_eligibility::filter::{
    Cause, EligibilityInput, EligibilityResult, EligibilityServices, eligible,
};
use crate::static_eligibility::optional::{
    DeploymentPreCallCheck, EnabledChecks, OptionalPreCallCheck, PreCallChecks, PreCallError,
};
use crate::static_eligibility::request::RequestFacts;

const MODEL: &str = "gpt";
const PRODUCTION: Option<&str> = Some("production");

fn support() -> Support {
    Support {
        resolves: true,
        supported: Some(vec!["response_format".to_owned()]),
    }
}

fn unsupporting() -> Support {
    Support {
        resolves: true,
        supported: Some(Vec::new()),
    }
}

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
            environment: PRODUCTION,
            enable_pre_call_checks: enable,
            drop_params: false,
        },
        &EligibilityServices {
            limits: &limits,
            params: support,
        },
    )
}

fn tool(kind: &str) -> Json {
    serde_json::from_str(&format!(r#"{{"type":"{kind}"}}"#)).unwrap_or(Json::Null)
}

fn message_facts(content: &str) -> RequestFacts {
    let mut facts = empty_facts();
    facts.messages = Some(vec![message(content)]);
    facts
}

fn pool(deployments: Vec<DeploymentSpec>) -> Candidates {
    Candidates::Pool(deployments)
}

#[derive(Clone, Copy, Debug)]
enum Behavior {
    EnvironmentAbsent,
    EnvironmentListed,
    EnvironmentUnlisted,
    WebSearchDropsUnsupported,
    WebSearchOtherToolKeepsAll,
    ContextKeepsWithinTheWindow,
    ContextDropsOverTheWindow,
    ContextRaisesWhenNoWindowFits,
    ContextSkipsWithoutMessages,
    ContextFiltersAResponsesInput,
    CompactionSkipsTheContextWindow,
    ParamsDropsUnsupportedResponseFormat,
    ParamsKeepsSupportedResponseFormat,
    ParamsSkipsWithoutResponseFormat,
    ParamsSkipsWithDropParams,
    ParamsKeepsAnUnresolvedProvider,
    ContextErrorWhenParamsRemoveTheRest,
    OrderKeepsTheSmallestGroup,
    OrderTakesTheTargetGroup,
    SingleDeploymentIsSelected,
    SingleBlockedDeploymentIsPaused,
    OptionalChecksUnionAndToggle,
    PreCallChecksPropagateTheFailure,
}

#[rstest]
#[case::environment_absent(Behavior::EnvironmentAbsent)]
#[case::environment_listed(Behavior::EnvironmentListed)]
#[case::environment_unlisted(Behavior::EnvironmentUnlisted)]
#[case::web_search_drops_unsupported(Behavior::WebSearchDropsUnsupported)]
#[case::web_search_other_tool_keeps_all(Behavior::WebSearchOtherToolKeepsAll)]
#[case::context_keeps_within(Behavior::ContextKeepsWithinTheWindow)]
#[case::context_drops_over(Behavior::ContextDropsOverTheWindow)]
#[case::context_raises_when_no_window_fits(Behavior::ContextRaisesWhenNoWindowFits)]
#[case::context_skips_without_messages(Behavior::ContextSkipsWithoutMessages)]
#[case::context_filters_responses_input(Behavior::ContextFiltersAResponsesInput)]
#[case::compaction_skips_context(Behavior::CompactionSkipsTheContextWindow)]
#[case::params_drop_unsupported(Behavior::ParamsDropsUnsupportedResponseFormat)]
#[case::params_keep_supported(Behavior::ParamsKeepsSupportedResponseFormat)]
#[case::params_skip_without_response_format(Behavior::ParamsSkipsWithoutResponseFormat)]
#[case::params_skip_with_drop_params(Behavior::ParamsSkipsWithDropParams)]
#[case::params_keep_unresolved(Behavior::ParamsKeepsAnUnresolvedProvider)]
#[case::context_error_after_params(Behavior::ContextErrorWhenParamsRemoveTheRest)]
#[case::order_smallest_group(Behavior::OrderKeepsTheSmallestGroup)]
#[case::order_target_group(Behavior::OrderTakesTheTargetGroup)]
#[case::single_selected(Behavior::SingleDeploymentIsSelected)]
#[case::single_blocked(Behavior::SingleBlockedDeploymentIsPaused)]
#[case::optional_checks(Behavior::OptionalChecksUnionAndToggle)]
#[case::pre_call_failure(Behavior::PreCallChecksPropagateTheFailure)]
fn static_eligibility_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::EnvironmentAbsent => environment_selection(&[("d1", "")], &["d1"], &[]),
        Behavior::EnvironmentListed => environment_selection(&[("d1", LISTED)], &["d1"], &[]),
        Behavior::EnvironmentUnlisted => environment_selection(
            &[("kept", LISTED), ("dropped", UNLISTED)],
            &["kept"],
            &[Cause::Environment {
                model_name: "gpt".to_owned(),
            }],
        ),
        Behavior::WebSearchDropsUnsupported => web_search_drops_unsupported(),
        Behavior::WebSearchOtherToolKeepsAll => web_search_other_tool_keeps_all(),
        Behavior::ContextKeepsWithinTheWindow => context_keeps_within(),
        Behavior::ContextDropsOverTheWindow => context_drops_over(),
        Behavior::ContextRaisesWhenNoWindowFits => context_raises_when_no_window_fits(),
        Behavior::ContextSkipsWithoutMessages => context_skips_without_messages(),
        Behavior::ContextFiltersAResponsesInput => context_filters_a_responses_input(),
        Behavior::CompactionSkipsTheContextWindow => compaction_skips_the_context_window(),
        Behavior::ParamsDropsUnsupportedResponseFormat => params_drops_unsupported(),
        Behavior::ParamsKeepsSupportedResponseFormat => params_keeps_supported(),
        Behavior::ParamsSkipsWithoutResponseFormat => params_skips_without_response_format(),
        Behavior::ParamsSkipsWithDropParams => params_skips_with_drop_params(),
        Behavior::ParamsKeepsAnUnresolvedProvider => params_keeps_an_unresolved_provider(),
        Behavior::ContextErrorWhenParamsRemoveTheRest => {
            context_error_when_params_remove_the_rest()
        }
        Behavior::OrderKeepsTheSmallestGroup => order_selection(
            &[
                ("one", Some(2)),
                ("two", Some(1)),
                ("three", Some(1)),
                ("none", None),
            ],
            None,
            &["two", "three"],
        ),
        Behavior::OrderTakesTheTargetGroup => {
            order_selection(&[("one", Some(2)), ("two", Some(1))], Some(2), &["one"])
        }
        Behavior::SingleDeploymentIsSelected => single_selection("live", "", true),
        Behavior::SingleBlockedDeploymentIsPaused => {
            single_selection("paused", r#"{"blocked":true}"#, false)
        }
        Behavior::OptionalChecksUnionAndToggle => optional_checks_union_and_toggle(),
        Behavior::PreCallChecksPropagateTheFailure => pre_call_checks_propagate_the_failure(),
    }
}

const LISTED: &str = r#"{"supported_environments":["production"]}"#;
const UNLISTED: &str = r#"{"supported_environments":["staging"]}"#;

fn environment_selection(rows: &[(&str, &str)], expected: &[&str], causes: &[Cause]) {
    let deployments: Vec<DeploymentSpec> = rows
        .iter()
        .map(|(id, info)| spec("gpt", id, "openai/gpt-4o", info))
        .collect();
    let result = run(
        pool(deployments),
        &RequestFacts::default(),
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), expected);
    assert_eq!(result.causes, causes);
}

fn web_search_drops_unsupported() {
    let mut facts = empty_facts();
    facts.tools = vec![tool("web_search")];
    let result = run(
        pool(vec![
            spec(
                "gpt",
                "no",
                "openai/gpt-4o",
                r#"{"supports_web_search":false}"#,
            ),
            spec("gpt", "default", "openai/gpt-4o", ""),
        ]),
        &facts,
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["default"]);
    assert_eq!(
        result.causes,
        [Cause::WebSearch {
            model_name: "gpt".to_owned()
        }]
    );
}

fn web_search_other_tool_keeps_all() {
    let mut facts = empty_facts();
    facts.tools = vec![tool("function")];
    let result = run(
        pool(vec![spec(
            "gpt",
            "no",
            "openai/gpt-4o",
            r#"{"supports_web_search":false}"#,
        )]),
        &facts,
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["no"]);
}

fn context_keeps_within() {
    let facts = RequestFacts {
        input_token_count: Some(5),
        ..message_facts("hello")
    };
    let result = run(
        pool(vec![
            spec("gpt", "fits", "openai/gpt-4o", r#"{"max_input_tokens":5}"#),
            spec("gpt", "over", "openai/gpt-4o", r#"{"max_input_tokens":4}"#),
            spec("gpt", "open", "openai/gpt-4o", ""),
        ]),
        &facts,
        true,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["fits", "open"]);
    assert_eq!(
        result.causes,
        [Cause::ContextWindow {
            model_name: "gpt".to_owned(),
            max_input_tokens: 4,
            input_tokens: 5,
        }]
    );
}

fn context_drops_over() {
    let facts = RequestFacts {
        input_token_count: Some(3),
        ..message_facts("hello")
    };
    let result = run(
        pool(vec![spec(
            "gpt",
            "small",
            "openai/gpt-4o",
            r#"{"max_input_tokens":2}"#,
        )]),
        &facts,
        true,
        &support(),
    );
    assert!(matches!(
        result,
        Err(EligibilityError::ContextWindow(_error))
    ));
}

fn context_raises_when_no_window_fits() {
    let facts = RequestFacts {
        input_token_count: Some(9),
        ..message_facts("hello")
    };
    let error = run(
        pool(vec![
            spec("gpt", "a", "openai/gpt-4o", r#"{"max_input_tokens":4}"#),
            spec("gpt", "b", "openai/gpt-4o", r#"{"max_input_tokens":5}"#),
        ]),
        &facts,
        true,
        &support(),
    )
    .unwrap_err();
    match error {
        EligibilityError::ContextWindow(window) => {
            assert!(window.causes.contains("Model=openai/gpt-4o"));
            assert!(window.causes.contains("Max Input Tokens=4"));
            assert!(window.causes.contains("Got=9"));
        }
        other => panic!("expected a context window error, got {other:?}"),
    }
}

fn context_skips_without_messages() {
    let mut facts = empty_facts();
    facts.input_token_count = Some(1000);
    let result = run(
        pool(vec![spec(
            "gpt",
            "tiny",
            "openai/gpt-4o",
            r#"{"max_input_tokens":1}"#,
        )]),
        &facts,
        true,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["tiny"]);
}

// An input-only Responses lookup enters the pre-call filters, and the window at the input count stays.
fn context_filters_a_responses_input() {
    let facts = RequestFacts {
        input: Some(Json::Text("hello world".to_owned())),
        input_token_count: Some(9),
        ..empty_facts()
    };
    let result = run(
        pool(vec![
            spec("gpt", "below", "openai/gpt-4o", r#"{"max_input_tokens":8}"#),
            spec("gpt", "edge", "openai/gpt-4o", r#"{"max_input_tokens":9}"#),
        ]),
        &facts,
        true,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["edge"]);
    assert_eq!(
        result.causes,
        [Cause::ContextWindow {
            model_name: "gpt".to_owned(),
            max_input_tokens: 8,
            input_tokens: 9,
        }]
    );
}

fn compaction_skips_the_context_window() {
    let facts = RequestFacts {
        input_token_count: Some(1000),
        compaction_pending: true,
        ..message_facts("hello")
    };
    let result = run(
        pool(vec![spec(
            "gpt",
            "tiny",
            "openai/gpt-4o",
            r#"{"max_input_tokens":1}"#,
        )]),
        &facts,
        true,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["tiny"]);
}

fn params_drops_unsupported() {
    let facts = RequestFacts {
        response_format: true,
        ..message_facts("hi")
    };
    let result = run(
        pool(vec![spec("gpt", "legacy", "openai/gpt-4", "")]),
        &facts,
        true,
        &unsupporting(),
    )
    .unwrap();
    assert!(result.deployments.is_empty());
    assert_eq!(
        result.causes,
        [Cause::UnsupportedParameter {
            model_name: "gpt".to_owned(),
            param: "response_format".to_owned(),
        }]
    );
}

fn params_keeps_supported() {
    let facts = RequestFacts {
        response_format: true,
        ..message_facts("hi")
    };
    let result = run(
        pool(vec![spec("gpt", "modern", "openai/gpt-4o", "")]),
        &facts,
        true,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["modern"]);
}

fn params_skips_without_response_format() {
    let facts = message_facts("hi");
    let result = run(
        pool(vec![spec("gpt", "legacy", "openai/gpt-4", "")]),
        &facts,
        true,
        &unsupporting(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["legacy"]);
}

fn params_skips_with_drop_params() {
    let facts = RequestFacts {
        response_format: true,
        ..message_facts("hi")
    };
    let limits = InfoLimits;
    let support = unsupporting();
    let result = eligible(
        EligibilityInput {
            model: MODEL,
            candidates: pool(vec![spec("gpt", "legacy", "openai/gpt-4", "")]),
            facts: &facts,
            environment: PRODUCTION,
            enable_pre_call_checks: true,
            drop_params: true,
        },
        &EligibilityServices {
            limits: &limits,
            params: &support,
        },
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["legacy"]);
}

// docs/deviations/B364.yaml: LiteLLM returns the whole pool unfiltered when the pre-call count is unavailable.
#[test]
fn a_missing_token_count_still_drops_an_unsupported_parameter() {
    let facts = RequestFacts {
        response_format: true,
        ..message_facts("hi")
    };
    let result = run(
        pool(vec![spec(
            "gpt",
            "windowed",
            "openai/gpt-4",
            r#"{"max_input_tokens":10}"#,
        )]),
        &facts,
        true,
        &unsupporting(),
    )
    .unwrap();
    assert!(result.deployments.is_empty());
}

// A provider list that names other params but not response_format still drops the candidate.
#[test]
fn a_supported_list_without_response_format_drops_the_candidate() {
    let support = Support {
        resolves: true,
        supported: Some(vec!["temperature".to_owned()]),
    };
    let facts = RequestFacts {
        response_format: true,
        ..message_facts("hi")
    };
    let result = run(
        pool(vec![spec("gpt", "partial", "openai/gpt-4o", "")]),
        &facts,
        true,
        &support,
    )
    .unwrap();
    assert!(result.deployments.is_empty());
}

fn params_keeps_an_unresolved_provider() {
    let facts = RequestFacts {
        response_format: true,
        ..message_facts("hi")
    };
    let unresolved = Support {
        resolves: false,
        supported: None,
    };
    let result = run(
        pool(vec![spec("gpt", "legacy", "openai/gpt-4", "")]),
        &facts,
        true,
        &unresolved,
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["legacy"]);
}

// LiteLLM raises the context error when every pre-call candidate is invalid and one left over its window.
fn context_error_when_params_remove_the_rest() {
    let facts = RequestFacts {
        input_token_count: Some(9),
        response_format: true,
        ..message_facts("hello")
    };
    let error = run(
        pool(vec![
            spec(
                "gpt",
                "windowed",
                "openai/gpt-4o",
                r#"{"max_input_tokens":4}"#,
            ),
            spec("gpt", "legacy", "openai/gpt-4", ""),
        ]),
        &facts,
        true,
        &unsupporting(),
    )
    .unwrap_err();
    match error {
        EligibilityError::ContextWindow(window) => {
            assert_eq!(
                window.causes,
                "Model=openai/gpt-4o, Max Input Tokens=4, Got=9"
            );
        }
        other => panic!("expected a context window error, got {other:?}"),
    }
}

fn order_selection(rows: &[(&str, Option<i64>)], target: Option<i64>, expected: &[&str]) {
    let mut deployments = Vec::new();
    for (id, order) in rows {
        let deployment = spec("gpt", id, "openai/gpt-4o", "");
        match order {
            Some(value) => deployments.push(with_param(
                deployment,
                "order",
                Json::Number(serde_json::Number::from(*value)),
            )),
            None => deployments.push(deployment),
        }
    }
    let mut facts = empty_facts();
    facts.target_order = target;
    let result = run(pool(deployments), &facts, false, &support()).unwrap();
    assert_eq!(ids(&result.deployments), expected);
}

fn single_selection(id: &str, info: &str, selected: bool) {
    let result = run(
        Candidates::Single(Box::new(spec("gpt", id, "openai/gpt-4o", info))),
        &RequestFacts::default(),
        false,
        &support(),
    );
    if selected {
        assert_eq!(ids(&result.unwrap().deployments), [id]);
    } else {
        assert!(matches!(result, Err(EligibilityError::Blocked { model }) if model == MODEL));
    }
}

fn optional_checks_union_and_toggle() {
    let mut enabled = EnabledChecks::default();
    enabled.add(&[
        OptionalPreCallCheck::DeploymentAffinity,
        OptionalPreCallCheck::PromptCaching,
        OptionalPreCallCheck::EnforceModelRateLimits,
        OptionalPreCallCheck::EncryptedContentAffinity,
    ]);
    enabled.add(&[OptionalPreCallCheck::DeploymentAffinity]);
    assert!(
        enabled
            .checks
            .contains(&OptionalPreCallCheck::DeploymentAffinity)
    );
    assert!(
        enabled
            .checks
            .contains(&OptionalPreCallCheck::PromptCaching)
    );
    assert!(
        enabled
            .checks
            .contains(&OptionalPreCallCheck::EnforceModelRateLimits)
    );
    assert!(
        enabled
            .checks
            .contains(&OptionalPreCallCheck::EncryptedContentAffinity)
    );
    enabled.replace(Some(&[OptionalPreCallCheck::RouterBudgetLimiting]));
    assert!(
        enabled
            .checks
            .contains(&OptionalPreCallCheck::DeploymentAffinity)
    );
    assert!(
        !enabled
            .checks
            .contains(&OptionalPreCallCheck::PromptCaching)
    );
    assert!(
        !enabled
            .checks
            .contains(&OptionalPreCallCheck::EnforceModelRateLimits)
    );
    assert!(
        !enabled
            .checks
            .contains(&OptionalPreCallCheck::EncryptedContentAffinity)
    );
    assert!(
        enabled
            .checks
            .contains(&OptionalPreCallCheck::RouterBudgetLimiting)
    );
    enabled.replace(None);
    assert!(
        enabled
            .checks
            .contains(&OptionalPreCallCheck::RouterBudgetLimiting)
    );
}

fn pre_call_checks_propagate_the_failure() {
    let check = Check {
        outcome: Err(PreCallError::Failed {
            message: "over budget".to_owned(),
        }),
    };
    let view: &dyn DeploymentPreCallCheck = &check;
    let deployment = spec("gpt", "d1", "openai/gpt-4o", "");
    let checks = [view];
    let result = PreCallChecks {
        deployment: &deployment,
        checks: &checks,
    }
    .run();
    assert_eq!(
        result,
        Err(PreCallError::Failed {
            message: "over budget".to_owned(),
        })
    );
}

#[tokio::test]
async fn async_pre_call_checks_record_the_cooldown_then_fail() {
    let check = Check {
        outcome: Err(PreCallError::RateLimited {
            deployment_id: "d1".to_owned(),
            status: 429,
        }),
    };
    let view: &dyn DeploymentPreCallCheck = &check;
    let cooldown = Cooldown::default();
    let deployment = spec("gpt", "d1", "openai/gpt-4o", "");
    let checks = [view];
    let result = PreCallChecks {
        deployment: &deployment,
        checks: &checks,
    }
    .run_async(&cooldown)
    .await;
    assert!(matches!(result, Err(PreCallError::RateLimited { .. })));
    let recorded = cooldown
        .recorded
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert_eq!(*recorded, [(String::from("d1"), 429)]);
}

// Only a rate limit records the cooldown; any other failure goes up with the cooldown untouched.
#[tokio::test]
async fn async_pre_call_checks_do_not_record_a_plain_failure() {
    let check = Check {
        outcome: Err(PreCallError::Failed {
            message: "over budget".to_owned(),
        }),
    };
    let view: &dyn DeploymentPreCallCheck = &check;
    let cooldown = Cooldown::default();
    let deployment = spec("gpt", "d1", "openai/gpt-4o", "");
    let checks = [view];
    assert_eq!(
        PreCallChecks {
            deployment: &deployment,
            checks: &checks,
        }
        .run_async(&cooldown)
        .await,
        Err(PreCallError::Failed {
            message: "over budget".to_owned(),
        })
    );
    let recorded = cooldown
        .recorded
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert!(recorded.is_empty());
}

#[rstest]
#[case::deployment_affinity("deployment_affinity", Some(OptionalPreCallCheck::DeploymentAffinity))]
#[case::responses_api_deployment_check(
    "responses_api_deployment_check",
    Some(OptionalPreCallCheck::ResponsesApiDeploymentCheck)
)]
#[case::session_affinity("session_affinity", Some(OptionalPreCallCheck::SessionAffinity))]
#[case::encrypted_content_affinity(
    "encrypted_content_affinity",
    Some(OptionalPreCallCheck::EncryptedContentAffinity)
)]
#[case::prompt_caching("prompt_caching", Some(OptionalPreCallCheck::PromptCaching))]
#[case::router_budget_limiting(
    "router_budget_limiting",
    Some(OptionalPreCallCheck::RouterBudgetLimiting)
)]
#[case::enforce_model_rate_limits(
    "enforce_model_rate_limits",
    Some(OptionalPreCallCheck::EnforceModelRateLimits)
)]
#[case::unknown("unknown", None)]
fn optional_check_names_map_to_their_checks(
    #[case] name: &str,
    #[case] expected: Option<OptionalPreCallCheck>,
) {
    assert_eq!(OptionalPreCallCheck::from_name(name), expected);
}
