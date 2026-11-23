mod forms;
mod store;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;

use async_trait::async_trait;
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
use betterlite_server::judge_targets::call::JudgeCall;
use betterlite_server::judge_targets::error::JudgeDispatchError;
use betterlite_server::judge_targets::sdk::JudgeSdk;
use betterlite_server::model_visibility::testing::{Row, Snapshot};
use betterlite_server::plugin_routes::routes::Identity;
use betterlite_server::shadow_eval_lifecycle::access::require_writer;
use betterlite_server::shadow_eval_lifecycle::body::{
    SHADOW_BODY_MAX_BYTES, ShadowBodyError, decode_shadow_body,
};
use betterlite_server::shadow_eval_lifecycle::error::LifecycleError;
use betterlite_server::shadow_eval_lifecycle::start::{StartDeps, start_shadow_eval};
use betterlite_server::shadow_eval_lifecycle::stop::stop_shadow_eval_job;
use betterlite_server::shadow_eval_validation::router_view::{
    AnthropicCredentials, ShadowEvalRouter,
};
use chrono::{DateTime, Utc};
use tokio::runtime::Builder;
use uuid::Uuid;

use self::forms::{Body, JobView, Role, StartStep, Step, StepOutcome, StopStep, role_name};
use self::store::ProbeStore;
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

pub use self::forms::{Input, Output};

struct Probe<'probe> {
    store: &'probe ProbeStore,
    fixture: &'probe Snapshot,
    settings: &'probe AnthropicSettings,
    cache: &'probe ProviderCache,
    resolution: &'probe ResolutionContext<'probe>,
    now: DateTime<Utc>,
    minted: std::vec::IntoIter<Uuid>,
}

struct NoSdk;

#[async_trait]
impl JudgeSdk for NoSdk {
    async fn acompletion(&self, _call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        Err(JudgeDispatchError {
            message: "the lifecycle probe never dispatches a judge call".to_owned(),
        })
    }
}

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
    let now = timestamp(input.now_millis)?;
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
    let info = Json::Object(JsonObject::new());
    let fixture =
        Snapshot::build(&probe_rows(input, &info), &input.model_group_alias).map_err(invalid)?;
    let store = ProbeStore::build(input)?;
    let minted: Vec<Uuid> = input
        .minted
        .iter()
        .map(String::as_str)
        .map(Uuid::parse_str)
        .collect::<Result<_, _>>()
        .map_err(invalid)?;
    let mut probe = Probe {
        store: &store,
        fixture: &fixture,
        settings: &settings,
        cache: &cache,
        resolution: &resolution,
        now,
        minted: minted.into_iter(),
    };
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(probe.step(step).await?);
    }
    Ok(Output {
        steps,
        jobs: store.states(),
    })
}

impl Probe<'_> {
    async fn step(&mut self, step: &Step) -> Result<StepOutcome, OperationError> {
        match step {
            Step::Parse(body) => parse_outcome(body),
            Step::Start(start) => self.start(start).await,
            Step::Stop(stop) => Ok(self.stop(stop).await),
        }
    }

    async fn start(&mut self, step: &StartStep) -> Result<StepOutcome, OperationError> {
        let identity = identity_of(step.role, step.user_id.as_deref());
        let store = match require_writer(
            &identity,
            "start a shadow eval",
            step.store.then_some(self.store),
        ) {
            Ok(store) => store,
            Err(error) => return Ok(failed(&error)),
        };
        let request = match decode_shadow_body(&body_bytes(&step.body)?) {
            Ok(request) => request,
            Err(error) => return Ok(failed(&LifecycleError::from(error))),
        };
        let provider = |_model: &str| -> Option<String> { None };
        let deps = StartDeps {
            router: Some(ShadowEvalRouter {
                catalog: &self.fixture.catalog,
                model_group_alias: &self.fixture.aliases,
                routing_groups: &self.fixture.groups,
                inferred_provider: &provider,
            }),
            sdk: &NoSdk,
            cache: self.cache,
            resolution: self.resolution,
            credentials: AnthropicCredentials {
                anthropic_key: None,
                api_key: None,
                settings: self.settings,
                secret_state: unconsulted(),
            },
        };
        match start_shadow_eval(
            store,
            deps,
            &request,
            identity.user_id.as_deref(),
            self.now,
            &mut self.minted,
        )
        .await
        {
            Ok(response) => Ok(StepOutcome::Succeeded {
                status: 201,
                job: JobView::of(response),
            }),
            Err(error) => Ok(failed(&error)),
        }
    }

    async fn stop(&self, step: &StopStep) -> StepOutcome {
        let identity = identity_of(step.role, step.user_id.as_deref());
        let store = match require_writer(
            &identity,
            "stop a shadow eval",
            step.store.then_some(self.store),
        ) {
            Ok(store) => store,
            Err(error) => return failed(&error),
        };
        match stop_shadow_eval_job(store, &step.job_id, identity.user_id.as_deref(), self.now).await
        {
            Ok(response) => StepOutcome::Succeeded {
                status: 200,
                job: JobView::of(response),
            },
            Err(error) => failed(&error),
        }
    }
}

fn parse_outcome(body: &Body) -> Result<StepOutcome, OperationError> {
    match decode_shadow_body(&body_bytes(body)?) {
        Ok(_request) => Ok(StepOutcome::Accepted),
        Err(ShadowBodyError::TooLarge { bytes, limit }) => {
            Ok(StepOutcome::OverLimit { bytes, limit })
        }
        Err(ShadowBodyError::Request(_invalid)) => Ok(StepOutcome::Invalid),
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

fn probe_rows<'input>(input: &'input Input, info: &'input Json) -> Vec<Row<'input>> {
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
