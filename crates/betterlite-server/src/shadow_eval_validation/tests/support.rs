use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use betterlite_anthropic::credentials::{ANTHROPIC_VERSION, AnthropicSettings};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::anthropic_credentials::DEFAULT_API_BASE;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
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

static NO_NAMES: BTreeSet<String> = BTreeSet::new();
static NO_ENVIRONMENT: BTreeMap<String, String> = BTreeMap::new();

pub(super) const KEYS: [TargetKey<'static>; 1] = [TargetKey {
    token: "key-1",
    team_id: Some("team-1"),
}];

pub(super) fn records() -> TargetRecords<'static> {
    TargetRecords {
        keys: &KEYS,
        teams: &["team-1"],
        users: &["user-1"],
    }
}

pub(super) fn parse(text: &str) -> StartShadowEvalRequest {
    parse_shadow_eval_request(text.as_bytes()).unwrap()
}

pub(super) fn validation_field(error: &ShadowEvalRequestError) -> &'static str {
    match error {
        ShadowEvalRequestError::Validation { field, .. } => field,
        ShadowEvalRequestError::Bounded(..)
        | ShadowEvalRequestError::Decode(..)
        | ShadowEvalRequestError::Field { .. }
        | ShadowEvalRequestError::NotAnObject => "not-a-validation-error",
    }
}

pub(super) const JUDGE: &str = "judge-model";
pub(super) const TIER: &str = "tier-model";
pub(super) const ROUTER: &str = "shadow-router";
pub(super) const JUDGE_MODEL: &str = "openai/gpt-4o-mini";
pub(super) const TIER_MODEL: &str = "anthropic/claude-sonnet-5";

pub(super) fn params(model: &str) -> JsonObject {
    JsonObject::from([("model".to_owned(), Json::Text(model.to_owned()))])
}

pub(super) fn router_params(default: &str) -> JsonObject {
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

pub(super) fn deployments() -> Vec<(&'static str, JsonObject)> {
    vec![
        (JUDGE, params(JUDGE_MODEL)),
        (TIER, params(TIER_MODEL)),
        (ROUTER, router_params(TIER)),
    ]
}

pub(super) fn auth(role: Option<&str>) -> UserAPIKeyAuth {
    let mut fields = BTreeMap::new();
    if let Some(name) = role {
        fields.insert("user_role".to_owned(), JsonData::String(name.to_owned()));
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(super) enum Credentials {
    None,
    Hosted(Vec<String>),
    ApiKey(String),
}

pub(super) fn validate(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    records: &TargetRecords<'_>,
) -> Result<(), ShadowEvalValidationError> {
    validate_with(request, deployments, records, &Credentials::None)
}

pub(super) fn validate_with(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    records: &TargetRecords<'_>,
    credentials: &Credentials,
) -> Result<(), ShadowEvalValidationError> {
    run_validation(
        request,
        deployments,
        JsonObject::new(),
        records,
        credentials,
    )
}

pub(super) fn validate_with_aliases(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    aliases: JsonObject,
    records: &TargetRecords<'_>,
) -> Result<(), ShadowEvalValidationError> {
    run_validation(request, deployments, aliases, records, &Credentials::None)
}

fn unconsulted_manager() -> SecretManagerReadState<'static> {
    SecretManagerReadState {
        client_exists: false,
        settings_exist: false,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

fn credential_state(credentials: &Credentials) -> (AnthropicSettings, SecretManagerReadState<'_>) {
    match credentials {
        Credentials::None => (AnthropicSettings::default(), unconsulted_manager()),
        Credentials::Hosted(keys) => (
            AnthropicSettings::default(),
            SecretManagerReadState {
                client_exists: true,
                settings_exist: true,
                access_mode: SecretAccessMode::ReadOnly,
                hosted_keys: Some(keys),
            },
        ),
        Credentials::ApiKey(key) => (
            AnthropicSettings {
                api_key: Some(SecretString::from(key.as_str())),
                auth_token: None,
                api_base: DEFAULT_API_BASE.to_owned(),
                use_bearer_for_custom_base: false,
                anthropic_version: ANTHROPIC_VERSION.to_owned(),
            },
            unconsulted_manager(),
        ),
    }
}

fn resolution_context(catalog: &ModelCatalog) -> ResolutionContext<'_> {
    ResolutionContext {
        catalog,
        known_model_names: &NO_NAMES,
        custom_providers: &NO_NAMES,
        environment: &NO_ENVIRONMENT,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

fn snapshot(deployments: &[(&str, JsonObject)], aliases: &JsonObject) -> Snapshot {
    let info = Json::Object(JsonObject::new());
    let rows: Vec<Row<'_>> = deployments
        .iter()
        .map(|(name, litellm_params)| Row {
            model_name: name,
            litellm_params,
            model_info: &info,
        })
        .collect();
    Snapshot::build(&rows, aliases).unwrap()
}

fn run_validation(
    request: &StartShadowEvalRequest,
    deployments: &[(&str, JsonObject)],
    aliases: JsonObject,
    records: &TargetRecords<'_>,
    credentials: &Credentials,
) -> Result<(), ShadowEvalValidationError> {
    let (settings, secret_state) = credential_state(credentials);
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let resolution = resolution_context(catalog.as_ref());
    let cache = ProviderCache::default();
    let sdk = NoSdk;
    let fixture = snapshot(deployments, &aliases);
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
            secret_state,
        },
        records: *records,
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
