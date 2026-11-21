use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::routes::CallType;
use betterlite_core::routes::catalog::RouteCatalog;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::{
    GlobalSkipFlags, GuardrailSkipFlags,
};
use betterlite_guardrails::guardrail_records::record::GuardrailSpanSink;
use betterlite_guardrails::guardrail_selection::gate::GuardrailSelection as _;
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation as _, TranslationCall, transform_user_key_metadata,
};
use betterlite_guardrails::unified_dispatch::registry::{
    ResponseKind, TranslationRegistry, ensure_request_metadata, infer_call_type,
};

use super::apply_record::{InvocationClock, RecordedGuardrail};
use super::other_adapters::mcp::MCP_CALL_TYPES;
use super::recorder::AppliedGuardrailRecorder;
use super::translation::UnifiedTranslation;
use crate::guardrails::bedrock_passthrough::event_stream::hook::PostCallSuccessHook;
use crate::guardrails::chat_guardrail_translation::rewrites::rebuild_ended_stream::ChunkAssembler;
use crate::guardrails::unified_chat::GuardrailToApply;
use crate::guardrails::unified_chat::stamps::StampSource;

// LiteLLM accepts the logging object's call type only for these four
const LOGGED_CALL_TYPES: [CallType; 4] = [
    CallType::PassThroughEndpoint,
    CallType::AllmPassthroughRoute,
    CallType::Ocr,
    CallType::Aocr,
];

// the hook's event, then the one it uses for an MCP call type
const PRE_CALL_EVENTS: [GuardrailEventHooks; 2] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PreMcpCall,
];

const DURING_CALL_EVENTS: [GuardrailEventHooks; 2] = [
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::DuringMcpCall,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GuardrailSkipDefaults {
    pub system: bool,
    pub tool: bool,
}

// LiteLLM's guardrail_to_apply: the guardrail, its CustomGuardrail settings and its own skip flags
pub struct GuardrailToRun<G> {
    pub guardrail: Arc<G>,
    pub base: Arc<GuardrailBase>,
    pub skip_flags: GuardrailSkipFlags,
}

pub struct HookRequest<'call> {
    pub data: &'call mut BTreeMap<String, GuardrailValue>,
    pub user_api_key_dict: &'call BTreeMap<String, GuardrailValue>,
    pub principal: &'call Principal,
    pub logging_obj: Option<&'call mut GuardrailLogTarget>,
    // LiteLLM's litellm_logging_obj.call_type, read only by the post-call fallback
    pub logging_call_type: Option<CallType>,
}

// what the pre-call and during-call hooks take
pub struct InputCall<'call, G> {
    pub guardrail: Option<&'call GuardrailToRun<G>>,
    pub request: HookRequest<'call>,
    pub call_type: CallType,
}

// the kind stands in for LiteLLM's isinstance checks on the response object
pub struct CallResponse {
    pub body: GuardrailValue,
    pub kind: ResponseKind,
}

type Translation<G, S, A, H> = UnifiedTranslation<RecordedGuardrail<G>, S, A, H>;

pub struct UnifiedGuardrailHooks<G, S, A, H, R>
where
    G: GuardrailContract + Send + Sync,
    H: PostCallSuccessHook + Sync,
{
    pub registry: TranslationRegistry<Translation<G, S, A, H>>,
    pub recorder: R,
    pub routes: Arc<RouteCatalog>,
    pub sink: Arc<dyn GuardrailSpanSink + Send + Sync>,
    pub clock: Arc<dyn InvocationClock>,
    pub skip_defaults: GuardrailSkipDefaults,
}

impl<G, S, A, H, R> UnifiedGuardrailHooks<G, S, A, H, R>
where
    G: GuardrailContract + Send + Sync,
    S: StampSource + Send + Sync,
    A: ChunkAssembler + Send + Sync,
    H: PostCallSuccessHook + Sync,
    R: AppliedGuardrailRecorder,
{
    pub async fn pre_call(&self, call: InputCall<'_, G>) -> Result<(), GuardrailFailure> {
        let guardrail = call.guardrail;
        let rewritten = self.rewrite_input(call, PRE_CALL_EVENTS).await?;
        if let (Some(data), Some(applied)) = (rewritten, guardrail)
            && !applied.base.settings.switches.records_own_information
        {
            let name = applied.base.settings.guardrail_name.as_deref();
            self.recorder.record(data, name);
        }
        Ok(())
    }

    // the during-call hook never writes the applied-guardrails entry
    pub async fn moderation(&self, call: InputCall<'_, G>) -> Result<(), GuardrailFailure> {
        self.rewrite_input(call, DURING_CALL_EVENTS)
            .await
            .map(|_data| ())
    }

    // the gate, the adapter lookup, the key metadata and the input rewrite; the data comes back when it ran
    async fn rewrite_input<'call>(
        &self,
        call: InputCall<'call, G>,
        [plain_event, mcp_event]: [GuardrailEventHooks; 2],
    ) -> Result<Option<&'call mut BTreeMap<String, GuardrailValue>>, GuardrailFailure> {
        let event_type = if MCP_CALL_TYPES.contains(&call.call_type) {
            mcp_event
        } else {
            plain_event
        };
        let Some(applied) = call.guardrail else {
            return Ok(None);
        };
        let Some(to_apply) = self.admitted(applied, &call.request, event_type)? else {
            return Ok(None);
        };
        let Some(entry) = self.registry.get(call.call_type) else {
            return Ok(None);
        };
        let HookRequest {
            data,
            user_api_key_dict,
            logging_obj,
            ..
        } = call.request;
        ensure_request_metadata(data, transform_user_key_metadata(Some(user_api_key_dict)));
        entry
            .translation
            .process_input_messages(data, &to_apply, logging_obj)
            .await?;
        Ok(Some(data))
    }

    // None when nothing ran, the response otherwise
    pub async fn post_call_success(
        &self,
        guardrail: Option<&GuardrailToRun<G>>,
        request: HookRequest<'_>,
        response: CallResponse,
    ) -> Result<Option<GuardrailValue>, GuardrailFailure> {
        let Some(applied) = guardrail else {
            return Ok(None);
        };
        let Some(to_apply) = self.admitted(applied, &request, GuardrailEventHooks::PostCall)?
        else {
            return Ok(None);
        };
        let Some(translation) = self.output_translation(&to_apply, &request, response.kind) else {
            return Ok(Some(response.body));
        };
        let original = response.body.clone();
        let call = TranslationCall {
            user_api_key_dict: Some(request.user_api_key_dict),
            request_data: Some(&mut *request.data),
            logging_obj: request.logging_obj,
            stream_transform_sink: None,
            deliver_ended_stream_rewrites: false,
        };
        let checked = translation
            .process_output_response(response.body, &to_apply, call)
            .await
            .map_err(|failure| with_original_response(failure, original))?;
        if !applied.base.settings.switches.records_own_information {
            self.recorder
                .record(request.data, to_apply.guardrail_name.as_deref());
        }
        Ok(Some(checked))
    }

    fn output_translation(
        &self,
        to_apply: &GuardrailToApply<RecordedGuardrail<G>>,
        request: &HookRequest<'_>,
        kind: ResponseKind,
    ) -> Option<&Translation<G, S, A, H>> {
        let route = match request.user_api_key_dict.get("request_route") {
            Some(GuardrailValue::Text(route)) => Some(route.as_str()),
            Some(_) | None => None,
        };
        let name = to_apply.guardrail_name.as_deref().unwrap_or("None");
        let route_label = route.unwrap_or("None");
        let Some(call_type) = self.resolve_call_type(route, kind, request.logging_call_type) else {
            tracing::warn!(
                "Guardrail '{name}' selected for route '{route_label}' but its call type could not be resolved; skipping post-call scanning. Add the route to API_ROUTE_TO_CALL_TYPES."
            );
            return None;
        };
        let Some(entry) = self.registry.get(call_type) else {
            let call_type_name = serde_json::to_string(&call_type).unwrap_or_default();
            tracing::warn!(
                "Guardrail '{name}' selected for route '{route_label}' but call type '{}' has no guardrail translation handler; skipping post-call scanning.",
                call_type_name.trim_matches('"')
            );
            return None;
        };
        Some(&entry.translation)
    }

    fn resolve_call_type(
        &self,
        route: Option<&str>,
        kind: ResponseKind,
        logged: Option<CallType>,
    ) -> Option<CallType> {
        route
            .and_then(|path| self.routes.call_types_for(path).ok().flatten())
            .and_then(|call_types| call_types.first().copied())
            .or_else(|| infer_call_type(kind))
            .or_else(|| logged.filter(|call_type| LOGGED_CALL_TYPES.contains(call_type)))
    }

    // the gate first, then the guardrail object the adapters read its name and skip flags from
    fn admitted(
        &self,
        applied: &GuardrailToRun<G>,
        request: &HookRequest<'_>,
        event_type: GuardrailEventHooks,
    ) -> Result<Option<GuardrailToApply<RecordedGuardrail<G>>>, GuardrailFailure> {
        let should_run = applied
            .base
            .should_run_guardrail(&*request.data, event_type, &BTreeSet::new())
            .map_err(|error| GuardrailFailure::Other(error.to_string()))?;
        let guardrail_name = applied.base.settings.guardrail_name.clone();
        if !should_run {
            tracing::debug!(
                "UnifiedLLMGuardrails: scanning disabled for {}",
                guardrail_name.as_deref().unwrap_or("None")
            );
            return Ok(None);
        }
        Ok(Some(GuardrailToApply {
            guardrail: Arc::new(RecordedGuardrail {
                guardrail: Arc::clone(&applied.guardrail),
                base: Arc::clone(&applied.base),
                sink: Arc::clone(&self.sink),
                clock: Arc::clone(&self.clock),
            }),
            guardrail_name,
            principal: request.principal.clone(),
            skip_flags: applied.skip_flags,
            global_flags: GlobalSkipFlags {
                skip_system_message: self.skip_defaults.system,
                skip_tool_message: self.skip_defaults.tool,
            },
        }))
    }
}

// the endpoint reports the upstream usage from the original response of a blocked call
fn with_original_response(failure: GuardrailFailure, original: GuardrailValue) -> GuardrailFailure {
    match failure {
        GuardrailFailure::ModifyResponse(mut details) => {
            if details.original_response.is_none() {
                details.original_response = Some(original);
            }
            GuardrailFailure::ModifyResponse(details)
        }
        other @ (GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Http { .. }
        | GuardrailFailure::HttpObject { .. }
        | GuardrailFailure::Other(_)) => other,
    }
}

mod tests;
