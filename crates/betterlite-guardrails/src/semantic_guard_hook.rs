pub mod error;
pub mod init;
pub mod scoring;

mod tests;
mod text;

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use indexmap::IndexMap;

use self::error::{SemanticGuardHookError, SemanticGuardSetupError};
use self::init::SemanticGuardArgs;
use self::scoring::SemanticRouteScorer;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::guardrail_records::messages::messages_for_call_type;
use crate::semantic_guard::semantic_guard_route_loading::build_routes;
use crate::semantic_guard::semantic_guard_route_loading::builder::{
    SemanticRouterBuilder, build_router,
};
use crate::semantic_guard::semantic_guard_route_loading::error::RouteLoadError;
use crate::semantic_guard::semantic_guard_route_loading::route::Route;
use crate::semantic_guard::semantic_guard_route_loading::sources::RouteSources;

pub const GUARDRAIL_PROVIDER: &str = "semantic_guard";
pub const BLOCKED_STATUS: u16 = 400;
pub const VIOLATION_TYPE: &str = "semantic_guard_violation";
pub const DEFAULT_EVENT_HOOK: &str = "pre_call";
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];

// the CustomGuardrail class switches of a guardrail that overrides both hooks and defines no apply_guardrail
pub const SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: false,
    uses_apply_guardrail_interface: false,
    deny_text_on_error: false,
};

pub struct SemanticGuardrail<R> {
    pub base: GuardrailBase,
    pub router: R,
    pub embedding_model: String,
    pub similarity_threshold: f64,
    pub on_flagged_action: String,
    pub route_count: usize,
}

// the loader reads template and YAML files, so it runs before the async router build
pub fn load_routes(args: &SemanticGuardArgs) -> Result<Vec<Route>, RouteLoadError> {
    build_routes(&RouteSources {
        route_templates: args.route_templates.clone(),
        custom_routes_file: args.custom_routes_file.clone(),
        custom_routes: args.custom_routes.clone(),
        global_threshold: args.similarity_threshold,
    })
}

impl<R> SemanticGuardrail<R>
where
    R: SemanticRouteScorer,
{
    pub async fn new<B>(
        args: SemanticGuardArgs,
        routes: Vec<Route>,
        builder: &B,
        strict_modes: bool,
    ) -> Result<Self, SemanticGuardSetupError<B::Error>>
    where
        B: SemanticRouterBuilder<Router = R>,
    {
        let base = GuardrailBase::new(guardrail_settings(&args), strict_modes)?;
        if routes.is_empty() {
            return Err(SemanticGuardSetupError::NoRoutes);
        }
        let route_count = routes.len();
        let router = build_router(builder, routes, args.similarity_threshold).await?;
        tracing::info!(
            "SemanticGuardrail '{}' initialized with {} routes, embedding_model={}, threshold={}",
            args.name,
            route_count,
            args.embedding_model,
            args.similarity_threshold
        );
        Ok(Self {
            base,
            router,
            embedding_model: args.embedding_model,
            similarity_threshold: args.similarity_threshold,
            on_flagged_action: args.on_flagged_action,
            route_count,
        })
    }

    pub async fn async_pre_call_hook(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        call_type: &str,
        tool_cache: &DualLocalCache,
    ) -> Result<(), GuardrailFailure> {
        let Some(text) = self.request_text(call_type, data, tool_cache)? else {
            return Ok(());
        };
        if text.is_empty() {
            return Ok(());
        }
        match self.check(&text, data).await? {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }

    pub async fn async_post_call_success_hook(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
        response: GuardrailValue,
    ) -> Result<GuardrailValue, GuardrailFailure> {
        let text = text::response_text(&response);
        if text.is_empty() {
            return Ok(response);
        }
        match self.check(&text, data).await? {
            Some(failure) => Err(failure),
            None => Ok(response),
        }
    }

    fn request_text(
        &self,
        call_type: &str,
        data: &BTreeMap<String, GuardrailValue>,
        tool_cache: &DualLocalCache,
    ) -> Result<Option<String>, SemanticGuardHookError<R::Error>> {
        let Some(messages) = messages_for_call_type(call_type, Some(data), tool_cache)
            .map_err(|source| SemanticGuardHookError::<R::Error>::Messages { source })?
        else {
            return Ok(None);
        };
        let text =
            text::user_text(&messages)
                .map_err(|source| SemanticGuardHookError::<R::Error>::Text { source })?;
        Ok(Some(text))
    }

    async fn check(
        &self,
        text: &str,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> Result<Option<GuardrailFailure>, SemanticGuardHookError<R::Error>> {
        let choices = self
            .router
            .score(text)
            .await
            .map_err(|source| SemanticGuardHookError::<R::Error>::Scoring { source })?;
        let Some(choice) = choices.first() else {
            return Ok(None);
        };
        let Some(route) = choice.name.as_deref().filter(|name| !name.is_empty()) else {
            return Ok(None);
        };
        Ok(Some(self.match_failure(
            route,
            choice.similarity_score,
            data,
        )))
    }

    fn match_failure(
        &self,
        route: &str,
        similarity_score: Option<f64>,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> GuardrailFailure {
        let guardrail_name = self.base.settings.guardrail_name.clone();
        let shown_name = guardrail_name.as_deref().unwrap_or("None");
        let message =
            format!("Request blocked by semantic guardrail '{shown_name}'. Matched route: {route}");
        let score_text =
            similarity_score.map_or_else(|| "None".to_owned(), |value| value.to_string());
        tracing::warn!(
            "SemanticGuard match: route={route}, score={score_text}, action={}",
            self.on_flagged_action
        );
        let score = similarity_score.map_or(GuardrailValue::Null, GuardrailValue::Decimal);
        if self.on_flagged_action == "passthrough" {
            let info = detection_info(route, score, guardrail_name);
            return self
                .base
                .passthrough_failure(&message, data, Some(info), None);
        }
        violation_detail(&message, route, score)
    }
}

fn detection_info(
    route: &str,
    score: GuardrailValue,
    guardrail_name: Option<String>,
) -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        (
            "route_name".to_owned(),
            GuardrailValue::Text(route.to_owned()),
        ),
        ("similarity_score".to_owned(), score),
        (
            "guardrail".to_owned(),
            guardrail_name.map_or(GuardrailValue::Null, GuardrailValue::Text),
        ),
    ])
}

fn violation_detail(message: &str, route: &str, score: GuardrailValue) -> GuardrailFailure {
    GuardrailFailure::HttpObject {
        status: BLOCKED_STATUS,
        detail: IndexMap::from([
            ("error".to_owned(), GuardrailValue::Text(message.to_owned())),
            ("route".to_owned(), GuardrailValue::Text(route.to_owned())),
            ("similarity_score".to_owned(), score),
            (
                "type".to_owned(),
                GuardrailValue::Text(VIOLATION_TYPE.to_owned()),
            ),
        ]),
    }
}

fn guardrail_settings(args: &SemanticGuardArgs) -> GuardrailSettings {
    let event_hook = match &args.event_hook {
        Some(LitellmParamsMode::One(name)) if name.is_empty() => None,
        Some(LitellmParamsMode::Many(names)) if names.is_empty() => None,
        configured => configured.clone(),
    }
    .unwrap_or(LitellmParamsMode::One(DEFAULT_EVENT_HOOK.to_owned()));
    GuardrailSettings {
        guardrail_name: Some(args.name.clone()),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: Some(event_hook),
        default_on: args.default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: SWITCHES,
    }
}
