use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::collector::TracebackPolicy;
use betterlite_config::spend_attribution::Settings as SpendAttributionSettings;
use betterlite_config::spend_storage::MissingSessionId;
use betterlite_logging::end_user::{
    CostTrackingService, EndUserCostTrackingSettings, get_end_user_id_for_cost_tracking,
};
use betterlite_logging::payload::deps::PayloadClock;
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::catalog::BundledCatalog;
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
use serde::Deserialize;
use tokio::sync::watch;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Payload {
        kwargs: JsonObject,
        #[serde(default)]
        response: ResponseInput,
        start_time: String,
        end_time: String,
        #[serde(default)]
        settings: SettingsInput,
        #[serde(default)]
        router: Option<RouterInput>,
        #[serde(default)]
        clock: f64,
        session_uuid: String,
    },
    EndUser {
        litellm_params: JsonObject,
        service: String,
        #[serde(default)]
        settings: SettingsInput,
    },
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseInput {
    #[default]
    Absent,
    Other,
    UnknownModelError,
    Mapping(JsonObject),
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct RouterInput {
    model_list: Vec<RouterDeployment>,
    // Router.model_names, which is_recognized_model reads
    recognized: Vec<String>,
    // Router.team_public_model_names
    team_public: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct RouterDeployment {
    model_name: String,
    #[serde(default)]
    litellm_params: JsonObject,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsInput {
    disable_end_user_cost_tracking: Option<bool>,
    #[serde(rename = "enable_end_user_cost_tracking_prometheus_only")]
    enable_prometheus_only: Option<bool>,
    store_bodies: bool,
    max_string_length: Option<i64>,
    turn_off_message_logging: bool,
    missing_session_id: Option<String>,
}

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
            } => payload(
                kwargs,
                response,
                start_time,
                end_time,
                settings,
                router.as_ref(),
                *clock,
                session_uuid,
            ),
            Input::EndUser {
                litellm_params,
                service,
                settings,
            } => end_user(litellm_params, service, settings),
        }
    }
}

fn payload(
    kwargs: &JsonObject,
    response: &ResponseInput,
    start_time: &str,
    end_time: &str,
    settings: &SettingsInput,
    router_input: Option<&RouterInput>,
    clock: f64,
    session_uuid: &str,
) -> HelperResult {
    let policy = payload_settings(settings)?;
    let completion_start_time = completion_start(kwargs)?;
    let mut deployment_catalog = DeploymentCatalog::default();
    let mut recognized = BTreeSet::new();
    let mut teams = BTreeSet::new();
    if let Some(router) = router_input {
        for deployment in &router.model_list {
            deployment_catalog.push(deployment_spec(deployment));
        }
        recognized.extend(router.recognized.iter().cloned());
        teams.extend(router.team_public.iter().cloned());
    }
    let aliases = AliasMap::new();
    let groups = RoutingGroupIndex::default();
    let inferred = |_: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &deployment_catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &inferred,
    };
    let mut bundled = BundledCatalog::default();
    let model_catalog = bundled.get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &model_catalog,
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let attribution_settings = SpendAttributionSettings::default();
    let router = router_input.map(|_| SpendRouter {
        catalog: Some(&lookup),
        settings: &attribution_settings,
        resolution: &resolution,
        recognized_groups: &recognized,
        team_public_names: &teams,
    });
    let injected_clock = CaseClock(clock);
    let (_sender, tracebacks) = watch::channel(TracebackPolicy { suppress: false });
    let request = PayloadRequest {
        kwargs,
        response: spend_response(response),
        start_time: timestamp(start_time)?,
        end_time: timestamp(end_time)?,
        completion_start_time,
        router: router.as_ref(),
        cache: None,
        clock: &injected_clock,
        session_uuid,
        tracebacks: &tracebacks,
    };
    let record = get_logging_payload(&request, &policy, &Utc)
        .map_err(|error| invalid(&error.to_string()))?;
    serde_json::to_value(&record).map_err(|error| invalid(&error.to_string()))
}

fn end_user(params: &JsonObject, service: &str, settings: &SettingsInput) -> HelperResult {
    let service = match service {
        "litellm_logging" => CostTrackingService::LitellmLogging,
        "prometheus" => CostTrackingService::Prometheus,
        other => return Err(invalid(&format!("unknown service {other}"))),
    };
    let policy = EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: settings.disable_end_user_cost_tracking,
        enable_prometheus_only: settings.enable_prometheus_only,
    };
    let resolved = get_end_user_id_for_cost_tracking(params, service, &policy)
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

fn spend_response<'response>(input: &'response ResponseInput) -> SpendResponse<'response> {
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
