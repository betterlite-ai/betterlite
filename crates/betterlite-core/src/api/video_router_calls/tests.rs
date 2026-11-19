#![cfg(test)]

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_error::proxy_model_not_found::route_endpoint;
use betterlite_openai::videos::VideoOptions;
use rstest::rstest;

use super::calls::{VideoRouterCalls, VideoRouterError};
use super::route::{VideoRoute, VideoRouteInput, VideoRouteView, plan_video_route};
use super::types::{VideoCallArgs, VideoCallKind, VideoReply};

#[derive(Debug)]
struct FakeView {
    recognized: Vec<&'static str>,
    team_models: Vec<(&'static str, &'static str, &'static str)>,
    team_public: Vec<&'static str>,
    model_names: Vec<&'static str>,
    pass_through: bool,
    default_or_pattern: bool,
    deployments: Vec<&'static str>,
}

impl VideoRouteView for FakeView {
    fn is_recognized_model(&self, model: &str) -> bool {
        self.recognized.iter().any(|candidate| *candidate == model)
    }

    fn map_team_model(&self, model: &str, team_id: &str) -> Option<String> {
        self.team_models
            .iter()
            .find(|entry| entry.0 == model && entry.1 == team_id)
            .map(|entry| entry.2.to_owned())
    }

    fn team_public_model_names_has(&self, model: &str) -> bool {
        self.team_public.iter().any(|candidate| *candidate == model)
    }

    fn model_names_has(&self, model: &str) -> bool {
        self.model_names.iter().any(|candidate| *candidate == model)
    }

    fn pass_through_all_models(&self) -> bool {
        self.pass_through
    }

    fn has_default_or_pattern(&self) -> bool {
        self.default_or_pattern
    }

    fn deployment_names_has(&self, model: &str) -> bool {
        self.deployments.iter().any(|candidate| *candidate == model)
    }
}

const fn empty_view() -> FakeView {
    FakeView {
        recognized: Vec::new(),
        team_models: Vec::new(),
        team_public: Vec::new(),
        model_names: Vec::new(),
        pass_through: false,
        default_or_pattern: false,
        deployments: Vec::new(),
    }
}

const fn input(kind: VideoCallKind, model: Option<&str>) -> VideoRouteInput<'_> {
    VideoRouteInput {
        kind,
        model,
        team_id: None,
        is_proxy_admin_without_team: false,
        has_api_key_or_base: false,
        user_model: None,
    }
}

fn call_args() -> VideoCallArgs {
    VideoCallArgs {
        fields: VideoOptions::new(),
        uploads: Vec::new(),
        specific_deployment: false,
    }
}

fn named(name: &str) -> VideoReply {
    VideoReply::Json(JsonValue::String(name.to_owned()))
}

struct EveryCall;

#[async_trait]
impl VideoRouterCalls for EveryCall {
    async fn avideo_generation(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_generation"))
    }

    async fn avideo_list(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_list"))
    }

    async fn avideo_status(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_status"))
    }

    async fn avideo_content(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(VideoReply::Bytes(b"avideo_content".to_vec()))
    }

    async fn avideo_remix(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_remix"))
    }

    async fn avideo_create_character(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_create_character"))
    }

    async fn avideo_get_character(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_get_character"))
    }

    async fn avideo_edit(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_edit"))
    }

    async fn avideo_extension(
        &self,
        _model: &str,
        _args: VideoCallArgs,
    ) -> Result<VideoReply, VideoRouterError> {
        Ok(named("avideo_extension"))
    }
}

#[rstest]
#[case::generation(VideoCallKind::Generation, "avideo_generation")]
#[case::list(VideoCallKind::List, "avideo_list")]
#[case::status(VideoCallKind::Status, "avideo_status")]
#[case::content(VideoCallKind::Content, "avideo_content")]
#[case::remix(VideoCallKind::Remix, "avideo_remix")]
#[case::create_character(VideoCallKind::CreateCharacter, "avideo_create_character")]
#[case::get_character(VideoCallKind::GetCharacter, "avideo_get_character")]
#[case::edit(VideoCallKind::Edit, "avideo_edit")]
#[case::extension(VideoCallKind::Extension, "avideo_extension")]
fn types_call_kind_names_the_nine_calls(#[case] kind: VideoCallKind, #[case] name: &str) {
    assert_eq!(kind.name(), name);
}

#[rstest]
#[case::generation(VideoCallKind::Generation)]
#[case::list(VideoCallKind::List)]
#[case::status(VideoCallKind::Status)]
#[case::content(VideoCallKind::Content)]
#[case::remix(VideoCallKind::Remix)]
#[case::create_character(VideoCallKind::CreateCharacter)]
#[case::get_character(VideoCallKind::GetCharacter)]
#[case::edit(VideoCallKind::Edit)]
#[case::extension(VideoCallKind::Extension)]
fn types_not_found_names_each_calls_route_path(#[case] kind: VideoCallKind) {
    let error = VideoRouterError::not_found(kind, "sora-2");
    let VideoRouterError::NotFound(inner) = &error else {
        panic!("not_found builds NotFound");
    };
    let path = route_endpoint(kind.name()).expect("every video call has a route path");
    let expected = format!("{path}: Invalid model name passed in model=sora-2.");
    assert!(inner.detail.error.starts_with(&expected));
    assert!(inner.retryable_with_model_read_through);
    assert_eq!(error.to_string(), inner.detail.error);
}

#[test]
fn types_mapped_error_displays_its_message() {
    let class = ErrorClass::InternalServer(Box::new(ErrorContext {
        message: "the router failed".to_owned(),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }));
    let error = VideoRouterError::Mapped {
        class: class.clone(),
        message: "the router failed".to_owned(),
    };
    assert_eq!(error.to_string(), "the router failed");
    let VideoRouterError::Mapped {
        class: kept,
        message,
    } = &error
    else {
        panic!("a mapped failure stays mapped");
    };
    assert_eq!(kept, &class);
    assert_eq!(message, "the router failed");
}

#[rstest]
#[case::generation(VideoCallKind::Generation)]
#[case::list(VideoCallKind::List)]
#[case::status(VideoCallKind::Status)]
#[case::content(VideoCallKind::Content)]
#[case::remix(VideoCallKind::Remix)]
#[case::create_character(VideoCallKind::CreateCharacter)]
#[case::get_character(VideoCallKind::GetCharacter)]
#[case::edit(VideoCallKind::Edit)]
#[case::extension(VideoCallKind::Extension)]
#[tokio::test]
async fn types_router_calls_answers_every_call(#[case] kind: VideoCallKind) {
    let calls: &dyn VideoRouterCalls = &EveryCall;
    let reply = match kind {
        VideoCallKind::Generation => calls.avideo_generation("sora-2", call_args()).await,
        VideoCallKind::List => calls.avideo_list("sora-2", call_args()).await,
        VideoCallKind::Status => calls.avideo_status("sora-2", call_args()).await,
        VideoCallKind::Content => calls.avideo_content("sora-2", call_args()).await,
        VideoCallKind::Remix => calls.avideo_remix("sora-2", call_args()).await,
        VideoCallKind::CreateCharacter => {
            calls.avideo_create_character("sora-2", call_args()).await
        }
        VideoCallKind::GetCharacter => calls.avideo_get_character("sora-2", call_args()).await,
        VideoCallKind::Edit => calls.avideo_edit("sora-2", call_args()).await,
        VideoCallKind::Extension => calls.avideo_extension("sora-2", call_args()).await,
    }
    .expect("the fake answers");
    let expected = if kind == VideoCallKind::Content {
        VideoReply::Bytes(b"avideo_content".to_vec())
    } else {
        named(kind.name())
    };
    assert_eq!(reply, expected);
}

#[rstest]
#[case::list(VideoCallKind::List)]
#[case::status(VideoCallKind::Status)]
#[case::content(VideoCallKind::Content)]
#[case::remix(VideoCallKind::Remix)]
#[case::create_character(VideoCallKind::CreateCharacter)]
#[case::get_character(VideoCallKind::GetCharacter)]
#[case::edit(VideoCallKind::Edit)]
#[case::extension(VideoCallKind::Extension)]
fn plan_no_model_eight_kinds_are_direct(#[case] kind: VideoCallKind) {
    let view = empty_view();
    let route = plan_video_route(&input(kind, None), Some(&view));
    assert_eq!(route, VideoRoute::Direct);
}

#[rstest]
#[case::list(VideoCallKind::List)]
#[case::status(VideoCallKind::Status)]
#[case::content(VideoCallKind::Content)]
#[case::remix(VideoCallKind::Remix)]
#[case::create_character(VideoCallKind::CreateCharacter)]
#[case::get_character(VideoCallKind::GetCharacter)]
#[case::edit(VideoCallKind::Edit)]
#[case::extension(VideoCallKind::Extension)]
fn plan_no_model_empty_name_is_direct(#[case] kind: VideoCallKind) {
    let view = empty_view();
    let route = plan_video_route(&input(kind, Some("")), Some(&view));
    assert_eq!(route, VideoRoute::Direct);
}

#[test]
fn plan_no_model_generation_is_not_found_without_a_name() {
    let view = empty_view();
    let route = plan_video_route(&input(VideoCallKind::Generation, None), Some(&view));
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: String::new()
        }
    );
}

#[test]
fn plan_no_model_generation_with_an_empty_name_is_not_found() {
    let view = empty_view();
    let route = plan_video_route(&input(VideoCallKind::Generation, Some("")), Some(&view));
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: String::new()
        }
    );
}

#[test]
fn plan_known_model_recognized_is_router() {
    let view = FakeView {
        recognized: vec!["sora-2"],
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("sora-2")),
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "sora-2".to_owned(),
            specific_deployment: false
        }
    );
}

#[test]
fn plan_known_model_team_mapping_uses_the_mapped_name() {
    let view = FakeView {
        team_models: vec![("internal-video", "team-1", "team/sora")],
        ..empty_view()
    };
    let route = plan_video_route(
        &VideoRouteInput {
            team_id: Some("team-1"),
            ..input(VideoCallKind::Generation, Some("internal-video"))
        },
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "team/sora".to_owned(),
            specific_deployment: false
        }
    );
}

#[test]
fn plan_known_model_team_mapping_needs_a_team_id() {
    let view = FakeView {
        team_models: vec![("internal-video", "team-1", "team/sora")],
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("internal-video")),
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: "internal-video".to_owned()
        }
    );
}

#[test]
fn plan_known_model_admin_public_name_is_router() {
    let view = FakeView {
        team_public: vec!["shared-video"],
        ..empty_view()
    };
    let route = plan_video_route(
        &VideoRouteInput {
            is_proxy_admin_without_team: true,
            ..input(VideoCallKind::Status, Some("shared-video"))
        },
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "shared-video".to_owned(),
            specific_deployment: false
        }
    );
}

#[test]
fn plan_known_model_admin_model_in_model_names_is_not_found() {
    let view = FakeView {
        team_public: vec!["shared-video"],
        model_names: vec!["shared-video"],
        ..empty_view()
    };
    let route = plan_video_route(
        &VideoRouteInput {
            is_proxy_admin_without_team: true,
            ..input(VideoCallKind::Status, Some("shared-video"))
        },
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: "shared-video".to_owned()
        }
    );
}

#[test]
fn plan_known_model_admin_model_outside_team_public_falls_through() {
    let view = empty_view();
    let route = plan_video_route(
        &VideoRouteInput {
            is_proxy_admin_without_team: true,
            ..input(VideoCallKind::Generation, Some("shared-video"))
        },
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: "shared-video".to_owned()
        }
    );
}

#[test]
fn plan_known_model_in_model_names_skips_the_unlisted_branches() {
    let view = FakeView {
        model_names: vec!["known-video"],
        pass_through: true,
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("known-video")),
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: "known-video".to_owned()
        }
    );
}

#[test]
fn plan_known_model_api_key_with_a_router_is_router() {
    let view = empty_view();
    let route = plan_video_route(
        &VideoRouteInput {
            has_api_key_or_base: true,
            ..input(VideoCallKind::Generation, Some("sora-2"))
        },
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "sora-2".to_owned(),
            specific_deployment: false
        }
    );
}

#[test]
fn plan_known_model_api_key_without_a_router_is_direct() {
    let route = plan_video_route(
        &VideoRouteInput {
            has_api_key_or_base: true,
            ..input(VideoCallKind::Generation, Some("sora-2"))
        },
        None,
    );
    assert_eq!(route, VideoRoute::Direct);
}

#[test]
fn plan_known_model_api_key_without_a_name_is_router() {
    let view = empty_view();
    let route = plan_video_route(
        &VideoRouteInput {
            has_api_key_or_base: true,
            ..input(VideoCallKind::Status, None)
        },
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: String::new(),
            specific_deployment: false
        }
    );
}

#[test]
fn plan_unknown_model_pass_through_is_direct() {
    let view = FakeView {
        pass_through: true,
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("mystery-video")),
        Some(&view),
    );
    assert_eq!(route, VideoRoute::Direct);
}

#[test]
fn plan_unknown_model_default_or_pattern_is_router() {
    let view = FakeView {
        default_or_pattern: true,
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("mystery-video")),
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "mystery-video".to_owned(),
            specific_deployment: false
        }
    );
}

#[test]
fn plan_unknown_model_deployment_name_is_router_for_that_deployment() {
    let view = FakeView {
        deployments: vec!["mystery-video"],
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("mystery-video")),
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "mystery-video".to_owned(),
            specific_deployment: true
        }
    );
}

#[test]
fn plan_unknown_model_default_or_pattern_beats_a_deployment_name() {
    let view = FakeView {
        default_or_pattern: true,
        deployments: vec!["mystery-video"],
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("mystery-video")),
        Some(&view),
    );
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "mystery-video".to_owned(),
            specific_deployment: false
        }
    );
}

#[test]
fn plan_unknown_model_pass_through_beats_a_default_and_a_deployment_name() {
    let view = FakeView {
        pass_through: true,
        default_or_pattern: true,
        deployments: vec!["mystery-video"],
        ..empty_view()
    };
    let route = plan_video_route(
        &input(VideoCallKind::Generation, Some("mystery-video")),
        Some(&view),
    );
    assert_eq!(route, VideoRoute::Direct);
}

#[rstest]
#[case::status(VideoCallKind::Status)]
#[case::content(VideoCallKind::Content)]
#[case::remix(VideoCallKind::Remix)]
#[case::create_character(VideoCallKind::CreateCharacter)]
#[case::get_character(VideoCallKind::GetCharacter)]
#[case::edit(VideoCallKind::Edit)]
#[case::extension(VideoCallKind::Extension)]
fn plan_unknown_model_fallback_kinds_are_router(#[case] kind: VideoCallKind) {
    let view = empty_view();
    let route = plan_video_route(&input(kind, Some("mystery-video")), Some(&view));
    assert_eq!(
        route,
        VideoRoute::Router {
            model: "mystery-video".to_owned(),
            specific_deployment: false
        }
    );
}

#[rstest]
#[case::generation(VideoCallKind::Generation)]
#[case::list(VideoCallKind::List)]
fn plan_unknown_model_generation_and_list_are_not_found(#[case] kind: VideoCallKind) {
    let view = empty_view();
    let route = plan_video_route(&input(kind, Some("mystery-video")), Some(&view));
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: "mystery-video".to_owned()
        }
    );
}

#[rstest]
#[case::generation(VideoCallKind::Generation)]
#[case::status(VideoCallKind::Status)]
fn plan_no_router_user_model_is_direct(#[case] kind: VideoCallKind) {
    let route = plan_video_route(
        &VideoRouteInput {
            user_model: Some("user/sora"),
            ..input(kind, Some("sora-2"))
        },
        None,
    );
    assert_eq!(route, VideoRoute::Direct);
}

#[test]
fn plan_no_router_user_model_without_a_name_is_direct() {
    let route = plan_video_route(
        &VideoRouteInput {
            user_model: Some("user/sora"),
            ..input(VideoCallKind::Generation, None)
        },
        None,
    );
    assert_eq!(route, VideoRoute::Direct);
}

#[test]
fn plan_no_router_without_a_user_model_is_not_found() {
    let route = plan_video_route(&input(VideoCallKind::Generation, Some("sora-2")), None);
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: "sora-2".to_owned()
        }
    );
}

#[test]
fn plan_no_router_without_a_name_is_not_found_without_a_name() {
    let route = plan_video_route(&input(VideoCallKind::Generation, None), None);
    assert_eq!(
        route,
        VideoRoute::NotFound {
            model: String::new()
        }
    );
}
