use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{Cooldown, FixedCounter, InfoLimits, Support, ids, spec, with_param};
use crate::deployment_catalog::types::Candidates;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::static_eligibility::error::EligibilityError;
use crate::static_eligibility::filter::{
    Cause, EligibilityInput, EligibilityResult, EligibilityServices, eligible,
};
use crate::static_eligibility::optional::{
    CooldownRecorder, DeploymentPreCallCheck, EnabledChecks, OptionalPreCallCheck, PreCallError,
    async_routing_strategy_pre_call_checks, routing_strategy_pre_call_checks,
};
use crate::static_eligibility::request::RequestFacts;
use crate::static_eligibility::tokens::{
    TokenCounter, TokenPayload, count_for_pre_call_checks, count_request_tokens,
};

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

fn message(content: &str) -> Json {
    serde_json::from_str(&format!(r#"{{"role":"user","content":"{content}"}}"#))
        .unwrap_or(Json::Null)
}

fn message_facts(content: &str) -> RequestFacts {
    RequestFacts {
        messages: Some(vec![message(content)]),
        ..RequestFacts::default()
    }
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
    CompactionSkipsTheContextWindow,
    ParamsDropsUnsupportedResponseFormat,
    ParamsKeepsSupportedResponseFormat,
    ParamsSkipsWithoutResponseFormat,
    ParamsSkipsWithDropParams,
    ParamsKeepsAnUnresolvedProvider,
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
#[case::compaction_skips_context(Behavior::CompactionSkipsTheContextWindow)]
#[case::params_drop_unsupported(Behavior::ParamsDropsUnsupportedResponseFormat)]
#[case::params_keep_supported(Behavior::ParamsKeepsSupportedResponseFormat)]
#[case::params_skip_without_response_format(Behavior::ParamsSkipsWithoutResponseFormat)]
#[case::params_skip_with_drop_params(Behavior::ParamsSkipsWithDropParams)]
#[case::params_keep_unresolved(Behavior::ParamsKeepsAnUnresolvedProvider)]
#[case::order_smallest_group(Behavior::OrderKeepsTheSmallestGroup)]
#[case::order_target_group(Behavior::OrderTakesTheTargetGroup)]
#[case::single_selected(Behavior::SingleDeploymentIsSelected)]
#[case::single_blocked(Behavior::SingleBlockedDeploymentIsPaused)]
#[case::optional_checks(Behavior::OptionalChecksUnionAndToggle)]
#[case::pre_call_failure(Behavior::PreCallChecksPropagateTheFailure)]
fn static_eligibility_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::EnvironmentAbsent => environment_absent(),
        Behavior::EnvironmentListed => environment_listed(),
        Behavior::EnvironmentUnlisted => environment_unlisted(),
        Behavior::WebSearchDropsUnsupported => web_search_drops_unsupported(),
        Behavior::WebSearchOtherToolKeepsAll => web_search_other_tool_keeps_all(),
        Behavior::ContextKeepsWithinTheWindow => context_keeps_within(),
        Behavior::ContextDropsOverTheWindow => context_drops_over(),
        Behavior::ContextRaisesWhenNoWindowFits => context_raises_when_no_window_fits(),
        Behavior::ContextSkipsWithoutMessages => context_skips_without_messages(),
        Behavior::CompactionSkipsTheContextWindow => compaction_skips_the_context_window(),
        Behavior::ParamsDropsUnsupportedResponseFormat => params_drops_unsupported(),
        Behavior::ParamsKeepsSupportedResponseFormat => params_keeps_supported(),
        Behavior::ParamsSkipsWithoutResponseFormat => params_skips_without_response_format(),
        Behavior::ParamsSkipsWithDropParams => params_skips_with_drop_params(),
        Behavior::ParamsKeepsAnUnresolvedProvider => params_keeps_an_unresolved_provider(),
        Behavior::OrderKeepsTheSmallestGroup => order_keeps_the_smallest_group(),
        Behavior::OrderTakesTheTargetGroup => order_takes_the_target_group(),
        Behavior::SingleDeploymentIsSelected => single_deployment_is_selected(),
        Behavior::SingleBlockedDeploymentIsPaused => single_blocked_deployment_is_paused(),
        Behavior::OptionalChecksUnionAndToggle => optional_checks_union_and_toggle(),
        Behavior::PreCallChecksPropagateTheFailure => pre_call_checks_propagate_the_failure(),
    }
}

fn environment_absent() {
    let result = run(
        pool(vec![spec("gpt", "d1", "openai/gpt-4o", "")]),
        &RequestFacts::default(),
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["d1"]);
    assert!(result.causes.is_empty());
}

fn environment_listed() {
    let result = run(
        pool(vec![spec(
            "gpt",
            "d1",
            "openai/gpt-4o",
            r#"{"supported_environments":["production"]}"#,
        )]),
        &RequestFacts::default(),
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["d1"]);
}

fn environment_unlisted() {
    let result = run(
        pool(vec![
            spec(
                "gpt",
                "kept",
                "openai/gpt-4o",
                r#"{"supported_environments":["production"]}"#,
            ),
            spec(
                "gpt",
                "dropped",
                "openai/gpt-4o",
                r#"{"supported_environments":["staging"]}"#,
            ),
        ]),
        &RequestFacts::default(),
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["kept"]);
    assert_eq!(
        result.causes,
        [Cause::Environment {
            model_name: "gpt".to_owned()
        }]
    );
}

fn web_search_drops_unsupported() {
    let facts = RequestFacts {
        tools: vec![tool("web_search")],
        ..RequestFacts::default()
    };
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
    let facts = RequestFacts {
        tools: vec![tool("function")],
        ..RequestFacts::default()
    };
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
    let facts = RequestFacts {
        input_token_count: Some(1000),
        ..RequestFacts::default()
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
        drop_params: true,
        ..message_facts("hi")
    };
    let result = run(
        pool(vec![spec("gpt", "legacy", "openai/gpt-4", "")]),
        &facts,
        true,
        &unsupporting(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["legacy"]);
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

fn order_keeps_the_smallest_group() {
    let deployments = vec![
        with_param(
            spec("gpt", "one", "openai/gpt-4o", ""),
            "order",
            Json::Number(serde_json::Number::from(2)),
        ),
        with_param(
            spec("gpt", "two", "openai/gpt-4o", ""),
            "order",
            Json::Number(serde_json::Number::from(1)),
        ),
        with_param(
            spec("gpt", "three", "openai/gpt-4o", ""),
            "order",
            Json::Number(serde_json::Number::from(1)),
        ),
        spec("gpt", "none", "openai/gpt-4o", ""),
    ];
    let result = run(
        pool(deployments),
        &RequestFacts::default(),
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["two", "three"]);
}

fn order_takes_the_target_group() {
    let deployments = vec![
        with_param(
            spec("gpt", "one", "openai/gpt-4o", ""),
            "order",
            Json::Number(serde_json::Number::from(2)),
        ),
        with_param(
            spec("gpt", "two", "openai/gpt-4o", ""),
            "order",
            Json::Number(serde_json::Number::from(1)),
        ),
    ];
    let facts = RequestFacts {
        target_order: Some(2),
        ..RequestFacts::default()
    };
    let result = run(pool(deployments), &facts, false, &support()).unwrap();
    assert_eq!(ids(&result.deployments), ["one"]);
}

fn single_deployment_is_selected() {
    let result = run(
        Candidates::Single(Box::new(spec("gpt", "live", "openai/gpt-4o", ""))),
        &RequestFacts::default(),
        false,
        &support(),
    )
    .unwrap();
    assert_eq!(ids(&result.deployments), ["live"]);
}

fn single_blocked_deployment_is_paused() {
    let result = run(
        Candidates::Single(Box::new(spec(
            "gpt",
            "paused",
            "openai/gpt-4o",
            r#"{"blocked":true}"#,
        ))),
        &RequestFacts::default(),
        false,
        &support(),
    );
    assert!(matches!(result, Err(EligibilityError::Blocked { model }) if model == MODEL));
}

fn optional_checks_union_and_toggle() {
    let mut enabled = EnabledChecks::default();
    enabled.add(&[
        OptionalPreCallCheck::DeploymentAffinity,
        OptionalPreCallCheck::PromptCaching,
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
    let check = Checked {
        outcome: Err(PreCallError::Failed {
            message: "over budget".to_owned(),
        }),
    };
    let view: &dyn DeploymentPreCallCheck = &check;
    let result = routing_strategy_pre_call_checks(&spec("gpt", "d1", "openai/gpt-4o", ""), &[view]);
    assert_eq!(
        result,
        Err(PreCallError::Failed {
            message: "over budget".to_owned(),
        })
    );
}

struct Checked {
    outcome: Result<(), PreCallError>,
}

#[async_trait::async_trait]
impl DeploymentPreCallCheck for Checked {
    fn check(&self, _deployment: &DeploymentSpec) -> Result<(), PreCallError> {
        self.outcome.clone()
    }

    async fn async_check(&self, _deployment: &DeploymentSpec) -> Result<(), PreCallError> {
        self.outcome.clone()
    }
}

#[tokio::test]
async fn async_pre_call_checks_record_the_cooldown_then_fail() {
    let check = Checked {
        outcome: Err(PreCallError::RateLimited {
            deployment_id: "d1".to_owned(),
            status: 429,
        }),
    };
    let view: &dyn DeploymentPreCallCheck = &check;
    let cooldown = Cooldown::default();
    let result = async_routing_strategy_pre_call_checks(
        &spec("gpt", "d1", "openai/gpt-4o", ""),
        &[view],
        &cooldown,
    )
    .await;
    assert!(matches!(result, Err(PreCallError::RateLimited { .. })));
    let recorded = cooldown
        .recorded
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert_eq!(*recorded, [(String::from("d1"), 429)]);
}

#[tokio::test]
async fn count_request_tokens_prefixes_the_anthropic_system_block() {
    let counter = FixedCounter { count: 0 };
    let messages: Vec<Json> = vec![message("hello")];
    let payload = TokenPayload {
        messages: Some(&messages),
        system: Some(&Json::Text("be brief".to_owned())),
        tools: &[],
    };
    assert_eq!(count_request_tokens(&payload, &counter).await.unwrap(), 2);
    let empty = TokenPayload {
        messages: Some(&messages),
        system: Some(&Json::Text(String::new())),
        tools: &[],
    };
    assert_eq!(count_request_tokens(&empty, &counter).await.unwrap(), 1);
}

#[tokio::test]
async fn count_request_tokens_rejects_a_request_without_messages() {
    let counter = FixedCounter { count: 0 };
    let payload = TokenPayload {
        messages: None,
        system: None,
        tools: &[],
    };
    assert!(matches!(
        count_request_tokens(&payload, &counter).await,
        Err(crate::static_eligibility::tokens::TokenCountError::NoInput)
    ));
}

#[tokio::test]
async fn count_for_pre_call_checks_needs_messages_and_a_declared_window() {
    let counter = FixedCounter { count: 7 };
    let messages: Vec<Json> = vec![message("hello")];
    let with_messages = TokenPayload {
        messages: Some(&messages),
        system: None,
        tools: &[],
    };
    let windowed = vec![spec(
        "gpt",
        "d1",
        "openai/gpt-4o",
        r#"{"max_input_tokens":100}"#,
    )];
    assert_eq!(
        count_for_pre_call_checks(MODEL, &windowed, &with_messages, &InfoLimits, &counter).await,
        Some(8)
    );
    let open = vec![spec("gpt", "d1", "openai/gpt-4o", "")];
    assert_eq!(
        count_for_pre_call_checks(MODEL, &open, &with_messages, &InfoLimits, &counter).await,
        None
    );
    assert_eq!(
        count_for_pre_call_checks(
            MODEL,
            &windowed,
            &TokenPayload {
                messages: None,
                system: None,
                tools: &[]
            },
            &InfoLimits,
            &counter
        )
        .await,
        None
    );
}

#[tokio::test]
async fn count_for_pre_call_checks_swallows_a_counter_failure() {
    struct Failing;
    #[async_trait::async_trait]
    impl TokenCounter for Failing {
        async fn count_messages(
            &self,
            _messages: &[betterlite_tokenizer::count::request::Message],
            _tools: &[Json],
        ) -> Result<usize, crate::static_eligibility::tokens::TokenCountError> {
            Err(crate::static_eligibility::tokens::TokenCountError::NoInput)
        }
    }
    let messages: Vec<Json> = vec![message("hello")];
    let payload = TokenPayload {
        messages: Some(&messages),
        system: None,
        tools: &[],
    };
    let windowed = vec![spec(
        "gpt",
        "d1",
        "openai/gpt-4o",
        r#"{"max_input_tokens":100}"#,
    )];
    assert_eq!(
        count_for_pre_call_checks(MODEL, &windowed, &payload, &InfoLimits, &Failing).await,
        None
    );
}

#[test]
fn cooldown_recorder_is_usable_directly() {
    let cooldown = Cooldown::default();
    cooldown.record_rate_limit("d1", 429);
    assert_eq!(
        *cooldown
            .recorded
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
        [(String::from("d1"), 429)]
    );
}
