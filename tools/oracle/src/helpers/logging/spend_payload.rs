mod input;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::collector::TracebackPolicy;
use betterlite_config::spend_attribution::Settings as SpendAttributionSettings;
use betterlite_config::spend_storage::MissingSessionId;
use betterlite_logging::end_user::{CostTrackingService, EndUserCostTrackingSettings};
// ast-grep-ignore: no-long-names
use betterlite_logging::end_user::get_end_user_id_for_cost_tracking;
use betterlite_logging::payload::deps::PayloadClock;
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_server::spend::payload::{
    CompletionStart, PayloadRequest, SpendPayloadSettings, SpendResponse, SpendRouter,
    get_logging_payload,
};
use chrono::{DateTime, Utc};
use input::{Input, ResponseInput, RouterDeployment, RouterInput, SettingsInput};
use tokio::sync::watch;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Payload {
                kwargs,
                response,
                start_time,
                end_time,
                settings,
                router,
                clock,
                session_uuid,
            } => payload(&PayloadCall {
                kwargs,
                response,
                start_time,
                end_time,
                settings,
                router_input: router.as_ref(),
                clock: *clock,
                session_uuid,
            }),
            Input::EndUser {
                litellm_params,
                service,
                settings,
            } => end_user(litellm_params, service, settings),
        }
    }
}

// The fields get_logging_payload's adapter reads, grouped so the call keeps one argument.
#[derive(Clone, Copy)]
struct PayloadCall<'call> {
    kwargs: &'call JsonObject,
    response: &'call ResponseInput,
    start_time: &'call str,
    end_time: &'call str,
    settings: &'call SettingsInput,
    router_input: Option<&'call RouterInput>,
    clock: f64,
    session_uuid: &'call str,
}

fn payload(call: &PayloadCall<'_>) -> HelperResult {
    let policy = payload_settings(call.settings)?;
    let completion_start_time = completion_start(call.kwargs)?;
    let router = OwnedRouter::build(call.router_input);
    record(
        call,
        Record {
            policy: &policy,
            completion_start_time,
            router: &router,
        },
    )
}

// The catalog and router settings get_logging_payload reads, owned so the request can borrow them.
struct OwnedRouter {
    deployment_catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
    model_catalog: Arc<ModelCatalog>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    recognized: BTreeSet<String>,
    teams: BTreeSet<String>,
    attribution_settings: SpendAttributionSettings,
}

impl OwnedRouter {
    fn build(input: Option<&RouterInput>) -> Self {
        let mut deployment_catalog = DeploymentCatalog::default();
        let mut recognized = BTreeSet::new();
        let mut teams = BTreeSet::new();
        if let Some(router) = input {
            for deployment in &router.model_list {
                deployment_catalog.push(deployment_spec(deployment));
            }
            recognized.extend(router.recognized.iter().cloned());
            teams.extend(router.team_public.iter().cloned());
        }
        Self {
            deployment_catalog,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
            model_catalog: BundledCatalog::default().get(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
            recognized,
            teams,
            attribution_settings: SpendAttributionSettings::default(),
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.model_catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

// The settings and router state a payload request is assembled from.
struct Record<'record> {
    policy: &'record SpendPayloadSettings,
    completion_start_time: CompletionStart,
    router: &'record OwnedRouter,
}

fn record(call: &PayloadCall<'_>, parts: Record<'_>) -> HelperResult {
    let Record {
        policy,
        completion_start_time,
        router,
    } = parts;
    let inferred = |_: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &router.deployment_catalog,
        model_group_alias: &router.aliases,
        routing_groups: &router.groups,
        inferred_provider: &inferred,
    };
    let resolution = router.resolution();
    let router_state = call.router_input.map(|_| SpendRouter {
        catalog: Some(&lookup),
        settings: &router.attribution_settings,
        resolution: &resolution,
        recognized_groups: &router.recognized,
        team_public_names: &router.teams,
    });
    let injected_clock = CaseClock(call.clock);
    let (_sender, tracebacks) = watch::channel(TracebackPolicy { suppress: false });
    let request = PayloadRequest {
        kwargs: call.kwargs,
        response: spend_response(call.response),
        start_time: timestamp(call.start_time)?,
        end_time: timestamp(call.end_time)?,
        completion_start_time,
        router: router_state.as_ref(),
        cache: None,
        clock: &injected_clock,
        session_uuid: call.session_uuid,
        tracebacks: &tracebacks,
    };
    let stored =
        get_logging_payload(&request, policy, &Utc).map_err(|error| invalid(&error.to_string()))?;
    serde_json::to_value(&stored).map_err(|error| invalid(&error.to_string()))
}

fn end_user(params: &JsonObject, service: &str, settings: &SettingsInput) -> HelperResult {
    let resolved_service = match service {
        "litellm_logging" => CostTrackingService::LitellmLogging,
        "prometheus" => CostTrackingService::Prometheus,
        other => return Err(invalid(&format!("unknown service {other}"))),
    };
    let policy = EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: settings.disable_end_user_cost_tracking,
        enable_prometheus_only: settings.enable_prometheus_only,
    };
    // ast-grep-ignore: no-long-names
    let resolved = get_end_user_id_for_cost_tracking(params, resolved_service, &policy)
        .map_err(|error| invalid(&error.to_string()))?;
    serde_json::to_value(resolved).map_err(|error| invalid(&error.to_string()))
}

fn payload_settings(input: &SettingsInput) -> Result<SpendPayloadSettings, Box<HelperError>> {
    let missing_session_id = match input.missing_session_id.as_deref() {
        None => None,
        Some("omit") => Some(MissingSessionId::Omit),
        Some("generate") => Some(MissingSessionId::Generate),
        Some("reject") => Some(MissingSessionId::Reject),
        Some(other) => return Err(invalid(&format!("unknown missing_session_id {other}"))),
    };
    Ok(SpendPayloadSettings {
        storage: SpendStoragePolicy {
            sanitize: SpendSanitizePolicy {
                store_bodies: input.store_bodies,
                max_string_length: input.max_string_length.unwrap_or(2048),
            },
            turn_off_message_logging: input.turn_off_message_logging,
        },
        missing_session_id,
        end_user: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: input.disable_end_user_cost_tracking,
            enable_prometheus_only: input.enable_prometheus_only,
        },
    })
}

fn completion_start(kwargs: &JsonObject) -> Result<CompletionStart, Box<HelperError>> {
    match kwargs.get("completion_start_time") {
        None => Ok(CompletionStart::Absent),
        Some(Json::Null) => Ok(CompletionStart::Null),
        Some(Json::Text(text)) => Ok(CompletionStart::At(timestamp(text)?)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(invalid("completion_start_time must be a string"))
        }
    }
}

const fn spend_response(input: &ResponseInput) -> SpendResponse<'_> {
    match input {
        ResponseInput::Absent => SpendResponse::Absent,
        ResponseInput::Other => SpendResponse::Other,
        ResponseInput::UnknownModelError => SpendResponse::UnknownModelError,
        ResponseInput::Mapping(fields) => SpendResponse::Mapping(fields),
    }
}

fn deployment_spec(deployment: &RouterDeployment) -> DeploymentSpec {
    let mut model_info = JsonObject::new();
    model_info.insert(
        "id".to_owned(),
        Json::Text(format!("deployment-{}", deployment.model_name)),
    );
    DeploymentSpec {
        model_name: deployment.model_name.clone(),
        litellm_params: deployment.litellm_params.clone(),
        model_info,
        extra: JsonObject::new(),
    }
}

fn timestamp(text: &str) -> Result<SpendTimestamp, Box<HelperError>> {
    DateTime::parse_from_rfc3339(text)
        .map(SpendTimestamp::Aware)
        .map_err(|error| invalid(&error.to_string()))
}

struct CaseClock(f64);

impl PayloadClock for CaseClock {
    fn now_epoch_seconds(&self) -> f64 {
        self.0
    }
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
