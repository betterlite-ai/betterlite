use betterlite_openai::openai_base_config::Object;

use super::super::route::{
    PassthroughRoute, PassthroughRouteInput, RouteFlag, plan_passthrough_route,
};
use super::support::{FakeView, RecordingBlocked, empty_view, route_input};
use crate::dispatch::tests::support::Trace;

#[test]
fn plan_api_key_with_router_is_router() {
    let trace = Trace::default();
    let view = FakeView::default();
    let blocked = RecordingBlocked {
        blocked: vec!["gpt-upstream"],
        trace: trace.clone(),
    };
    let input = PassthroughRouteInput {
        has_api_key_or_base: RouteFlag(true),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
    assert!(trace.steps().is_empty());
}

#[test]
fn plan_api_key_without_router_is_direct() {
    let blocked = RecordingBlocked {
        blocked: vec!["gpt-upstream"],
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        has_api_key_or_base: RouteFlag(true),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, None, &blocked),
        PassthroughRoute::Direct
    );
}

#[test]
fn plan_user_config_is_user_config() {
    let view = FakeView::default();
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        has_user_config: RouteFlag(true),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::UserConfig
    );
    assert_eq!(
        plan_passthrough_route(&input, None, &blocked),
        PassthroughRoute::UserConfig
    );
}

#[test]
fn plan_api_key_beats_user_config() {
    let view = FakeView::default();
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        has_api_key_or_base: RouteFlag(true),
        has_user_config: RouteFlag(true),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
}

#[test]
fn plan_user_config_beats_override() {
    let view = FakeView::default();
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        has_user_config: RouteFlag(true),
        router_settings_override: Some(Object::new()),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::UserConfig
    );
}

#[test]
fn plan_override_with_router_is_router() {
    let trace = Trace::default();
    let view = FakeView {
        deployment_names: vec!["gpt-upstream"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: vec!["gpt-upstream"],
        trace: trace.clone(),
    };
    let input = PassthroughRouteInput {
        router_settings_override: Some(Object::new()),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
    assert!(trace.steps().is_empty());
}

#[test]
fn plan_override_without_router_is_direct() {
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        router_settings_override: Some(Object::new()),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, None, &blocked),
        PassthroughRoute::Direct
    );
}

#[test]
fn plan_without_router_is_direct() {
    let trace = Trace::default();
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: trace.clone(),
    };
    let input = route_input("gpt-upstream");
    assert_eq!(
        plan_passthrough_route(&input, None, &blocked),
        PassthroughRoute::Direct
    );
    assert!(trace.steps().is_empty());
}

#[test]
fn plan_blocked_model_is_blocked() {
    let trace = Trace::default();
    let view = FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: vec!["gpt-upstream"],
        trace: trace.clone(),
    };
    let input = route_input("gpt-upstream");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Blocked
    );
    assert_eq!(trace.steps(), ["blocked:gpt-upstream:"]);
}

#[test]
fn plan_blocked_check_gets_team_id() {
    let trace = Trace::default();
    let view = FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: vec!["team-model"],
        trace: trace.clone(),
    };
    let input = PassthroughRouteInput {
        team_id: Some(String::from("team-1")),
        ..route_input("team-model")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Blocked
    );
    assert_eq!(trace.steps(), ["blocked:team-model:team-1"]);
}

#[test]
fn plan_blocked_skips_empty_model() {
    let trace = Trace::default();
    let view = FakeView::default();
    let blocked = RecordingBlocked {
        blocked: vec![""],
        trace: trace.clone(),
    };
    let input = route_input("");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::NotFound
    );
    assert!(trace.steps().is_empty());
}

#[test]
fn plan_blocked_before_team_mapping() {
    let view = FakeView {
        team_map: vec![("team-model", "team-1", "global-model")],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: vec!["team-model"],
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        team_id: Some(String::from("team-1")),
        ..route_input("team-model")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Blocked
    );
}

#[test]
fn plan_team_model_is_router() {
    let view = FakeView {
        team_map: vec![("team-model", "team-1", "global-model")],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        team_id: Some(String::from("team-1")),
        ..route_input("team-model")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
}

#[test]
fn plan_recognized_model_is_router() {
    let view = FakeView {
        recognized: RouteFlag(true),
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("gpt-upstream");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
}

#[test]
fn plan_admin_team_public_model_is_router() {
    let view = FakeView {
        team_public: vec!["secret-model"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        is_proxy_admin_without_team: RouteFlag(true),
        ..route_input("secret-model")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
}

#[test]
fn plan_admin_model_in_names_is_not_team_public_router() {
    let view = FakeView {
        team_public: vec!["gpt-upstream"],
        model_names: vec!["gpt-upstream"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = PassthroughRouteInput {
        is_proxy_admin_without_team: RouteFlag(true),
        ..route_input("gpt-upstream")
    };
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::NotFound
    );
}

#[test]
fn plan_team_public_model_needs_admin() {
    let view = FakeView {
        team_public: vec!["secret-model"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("secret-model");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::NotFound
    );
}

#[test]
fn plan_pass_through_all_is_direct() {
    let view = FakeView {
        pass_through_all: RouteFlag(true),
        default_or_pattern: RouteFlag(true),
        deployment_names: vec!["anthropic.claude-3"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("anthropic.claude-3");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Direct
    );
}

#[test]
fn plan_default_or_pattern_is_router() {
    let view = FakeView {
        default_or_pattern: RouteFlag(true),
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("anthropic.claude-3");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
}

#[test]
fn plan_deployment_name_is_specific() {
    let view = FakeView {
        deployment_names: vec!["deployment-7"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("deployment-7");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: true,
        }
    );
}

#[test]
fn plan_wildcard_beats_deployment() {
    let view = FakeView {
        default_or_pattern: RouteFlag(true),
        deployment_names: vec!["deployment-7"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("deployment-7");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::Router {
            specific_deployment: false,
        }
    );
}

#[test]
fn plan_unknown_model_is_not_found() {
    let view = FakeView::default();
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("missing");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::NotFound
    );
}

#[test]
fn plan_model_in_names_is_not_found() {
    let view = FakeView {
        model_names: vec!["gpt-upstream"],
        ..empty_view()
    };
    let blocked = RecordingBlocked {
        blocked: Vec::new(),
        trace: Trace::default(),
    };
    let input = route_input("gpt-upstream");
    assert_eq!(
        plan_passthrough_route(&input, Some(&view), &blocked),
        PassthroughRoute::NotFound
    );
}
