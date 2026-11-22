use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_secret_managers::secret_helpers::SecretManagerReadState;
use betterlite_secret_managers::secret_types::SecretAccessMode;
use secrecy::SecretString;

use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::JudgeDispatchError;
use crate::judge_targets::sdk::JudgeSdk;
use crate::model_visibility::testing::{Row, Snapshot};
use crate::shadow_eval_validation::error::{ShadowEvalRequestError, ShadowEvalValidationError};
use crate::shadow_eval_validation::request::{StartShadowEvalRequest, parse_shadow_eval_request};
use crate::shadow_eval_validation::router_view::{AnthropicCredentials, ShadowEvalRouter};
use crate::shadow_eval_validation::start::{
    StartContext, TargetKey, TargetRecords, validate_start,
};

pub const KEYS: [TargetKey<'static>; 1] = [TargetKey {
    token: "key-1",
    team_id: Some("team-1"),
}];

pub fn records() -> TargetRecords<'static> {
    TargetRecords {
        keys: &KEYS,
        teams: &["team-1"],
        users: &["user-1"],
    }
}

pub fn empty_records() -> TargetRecords<'static> {
    TargetRecords {
        keys: &[],
        teams: &[],
        users: &[],
    }
}

pub fn parse(text: &str) -> StartShadowEvalRequest {
    parse_shadow_eval_request(text.as_bytes()).unwrap()
}

pub fn validation_field(error: ShadowEvalRequestError) -> &'static str {
    match error {
        ShadowEvalRequestError::Validation { field, .. } => field,
        ShadowEvalRequestError::Bounded(..)
        | ShadowEvalRequestError::Decode(..)
        | ShadowEvalRequestError::Field { .. }
        | ShadowEvalRequestError::NotAnObject => "not-a-validation-error",
    }
}

pub const JUDGE: &str = "judge-model";
pub const TIER: &str = "tier-model";
pub const ROUTER: &str = "shadow-router";
pub const JUDGE_MODEL: &str = "openai/gpt-4o-mini";
pub const TIER_MODEL: &str = "anthropic/claude-sonnet-5";

pub fn params(model: &str) -> JsonObject {
    JsonObject::from([("model".to_owned(), Json::Text(model.to_owned()))])
}

pub fn router_params(default: &str) -> JsonObject {
    JsonObject::from([
        (
            "model".to_owned(),
            Json::Text("auto_router/complexity_router".to_owned()),
        ),
        (
            "complexity_router_default_model".to_owned(),
            Json::Text(default.to_owned()),
        ),
    ])
}

pub fn deployments() -> Vec<(&'static str, JsonObject)> {
    vec![
        (JUDGE, params(JUDGE_MODEL)),
        (TIER, params(TIER_MODEL)),
        (ROUTER, router_params(TIER)),
    ]
}

pub fn auth(role: Option<&str>) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(name) = role {
        fields.insert("user_role".to_owned(), JsonData::String(name.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub fn validate(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    records: &TargetRecords<'_>,
) -> Result<(), ShadowEvalValidationError> {
    run_validation(
        request,
        deployments,
        records,
        AnthropicSettings::default(),
        unconsulted_manager(),
    )
}

fn unconsulted_manager() -> SecretManagerReadState<'static> {
    SecretManagerReadState {
        client_exists: false,
        settings_exist: false,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

pub fn validate_with_hosted_keys(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    records: &TargetRecords<'_>,
    hosted_keys: &[String],
) -> Result<(), ShadowEvalValidationError> {
    let manager = SecretManagerReadState {
        client_exists: true,
        settings_exist: true,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: Some(hosted_keys),
    };
    run_validation(
        request,
        deployments,
        records,
        AnthropicSettings::default(),
        manager,
    )
}

pub fn validate_with_api_key(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    records: &TargetRecords<'_>,
    api_key: &str,
) -> Result<(), ShadowEvalValidationError> {
    let settings = AnthropicSettings {
        api_key: Some(SecretString::from(api_key)),
        ..AnthropicSettings::default()
    };
    run_validation(
        request,
        deployments,
        records,
        settings,
        unconsulted_manager(),
    )
}

fn run_validation(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    records: &TargetRecords<'_>,
    settings: AnthropicSettings,
    manager: SecretManagerReadState<'_>,
) -> Result<(), ShadowEvalValidationError> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let cache = ProviderCache::default();
    let sdk = NoSdk;
    let info = Json::Object(JsonObject::new());
    let rows: Vec<Row<'_>> = deployments
        .iter()
        .map(|(name, litellm_params)| Row {
            model_name: *name,
            litellm_params,
            model_info: &info,
        })
        .collect();
    let fixture = Snapshot::build(&rows, &JsonObject::new()).unwrap();
    let provider = |_model: &str| -> Option<String> { None };
    let context = StartContext {
        router: Some(ShadowEvalRouter {
            catalog: &fixture.catalog,
            model_group_alias: &fixture.aliases,
            routing_groups: &fixture.groups,
            inferred_provider: &provider,
        }),
        sdk: &sdk,
        cache: &cache,
        resolution: &resolution,
        credentials: AnthropicCredentials {
            anthropic_key: None,
            api_key: None,
            settings: &settings,
            secret_state: manager,
        },
        records: TargetRecords {
            keys: records.keys,
            teams: records.teams,
            users: records.users,
        },
    };
    validate_start(request, &context)
}

pub fn validate_without_router(
    request: &StartShadowEvalRequest,
    records: &TargetRecords<'_>,
) -> Result<(), ShadowEvalValidationError> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let cache = ProviderCache::default();
    let sdk = NoSdk;
    let settings = AnthropicSettings::default();
    let context = StartContext {
        router: None,
        sdk: &sdk,
        cache: &cache,
        resolution: &resolution,
        credentials: AnthropicCredentials {
            anthropic_key: None,
            api_key: None,
            settings: &settings,
            secret_state: SecretManagerReadState {
                client_exists: false,
                settings_exist: false,
                access_mode: SecretAccessMode::ReadOnly,
                hosted_keys: None,
            },
        },
        records: TargetRecords {
            keys: records.keys,
            teams: records.teams,
            users: records.users,
        },
    };
    validate_start(request, &context)
}

struct NoSdk;

#[async_trait]
impl JudgeSdk for NoSdk {
    async fn acompletion(&self, _call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        Err(JudgeDispatchError {
            message: "validation dispatches no judge call".to_owned(),
        })
    }
}
