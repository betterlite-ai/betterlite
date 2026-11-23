mod forms;
mod output;
mod probe;
mod store;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::vec::IntoIter;

use betterlite_anthropic::credentials::AnthropicSettings;
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
use betterlite_server::model_visibility::testing::{Row, Snapshot};
use betterlite_server::plugin_routes::routes::Identity;
use betterlite_server::shadow_eval_lifecycle::body::{
    SHADOW_BODY_MAX_BYTES, ShadowBodyError, decode_shadow_body,
};
use betterlite_server::shadow_eval_lifecycle::error::LifecycleError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use uuid::Uuid;

use self::forms::{Body, Payload, Role, role_name};
use self::output::{OutputFields, StepOutcome};
use self::probe::{Fixture, Probe};
use self::store::ProbeStore;
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the probe enums in mod.rs hold this payload, and the box keeps their largest variant small
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<OutputFields>);

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(invalid)?;
    let output = runtime.block_on(scenario(input))?;
    Ok(ProbeOutput::ShadowEvalLifecycle(output))
}

async fn scenario(input: &Input) -> Result<Output, OperationError> {
    let payload = &input.0;
    let now = timestamp(payload.now_millis)?;
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let no_names = BTreeSet::new();
    let no_environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &no_names,
        custom_providers: &no_names,
        environment: &no_environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let cache = ProviderCache::default();
    let settings = AnthropicSettings::default();
    let fixture = fixture_of(payload, &Json::Object(JsonObject::new()))?;
    let store = ProbeStore::build(payload)?;
    let mut probe = Probe {
        store: &store,
        fixture,
        settings: &settings,
        cache: &cache,
        resolution: &resolution,
        now,
        minted: minted_ids(payload)?,
    };
    let mut steps = Vec::with_capacity(payload.steps.len());
    for step in &payload.steps {
        steps.push(probe.step(step).await?);
    }
    Ok(Output(Box::new(OutputFields {
        steps,
        jobs: store.states(),
    })))
}

// a snapshot's boxed provider closure is neither Send nor Sync, so it is dropped before any await
fn fixture_of(payload: &Payload, info: &Json) -> Result<Fixture, OperationError> {
    let snapshot =
        Snapshot::build(&probe_rows(payload, info), &payload.model_group_alias).map_err(invalid)?;
    drop(snapshot.provider);
    Ok(Fixture {
        catalog: snapshot.catalog,
        aliases: snapshot.aliases,
        groups: snapshot.groups,
    })
}

fn parse_outcome(body: &Body) -> Result<StepOutcome, OperationError> {
    match decode_shadow_body(&body_bytes(body)?) {
        Ok(_) => Ok(StepOutcome::Accepted),
        Err(ShadowBodyError::TooLarge { bytes, limit }) => {
            Ok(StepOutcome::OverLimit { bytes, limit })
        }
        Err(ShadowBodyError::Request(_)) => Ok(StepOutcome::Invalid),
    }
}

fn body_bytes(body: &Body) -> Result<Vec<u8>, OperationError> {
    let mut bytes = body.text.clone().into_bytes();
    let total = body.pad_to.unwrap_or(bytes.len());
    if total < bytes.len() || total > SHADOW_BODY_MAX_BYTES.saturating_add(1) {
        return Err(invalid(
            "pad_to must start at the body length and stay within the JSON limit",
        ));
    }
    bytes.resize(total, b' ');
    Ok(bytes)
}

fn failed(error: &LifecycleError) -> StepOutcome {
    StepOutcome::Failed {
        status: error.status_code(),
        detail: error.to_string(),
    }
}

fn identity_of(role: Role, user_id: Option<&str>) -> Identity {
    Identity {
        user_id: user_id.map(str::to_owned),
        user_role: role_name(role).map(str::to_owned),
    }
}

fn probe_rows<'payload>(input: &'payload Payload, info: &'payload Json) -> Vec<Row<'payload>> {
    input
        .router
        .iter()
        .map(|deployment| Row {
            model_name: &deployment.model_name,
            litellm_params: &deployment.litellm_params,
            model_info: info,
        })
        .collect()
}

fn minted_ids(payload: &Payload) -> Result<IntoIter<Uuid>, OperationError> {
    let minted: Vec<Uuid> = payload
        .minted
        .iter()
        .map(String::as_str)
        .map(Uuid::parse_str)
        .collect::<Result<_, _>>()
        .map_err(invalid)?;
    Ok(minted.into_iter())
}

const fn unconsulted() -> SecretManagerReadState<'static> {
    SecretManagerReadState {
        client_exists: false,
        settings_exist: false,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

pub(super) fn timestamp(millis: i64) -> Result<DateTime<Utc>, OperationError> {
    DateTime::from_timestamp_millis(millis)
        .ok_or_else(|| invalid("a millisecond timestamp is not a valid instant"))
}

pub(super) fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}
