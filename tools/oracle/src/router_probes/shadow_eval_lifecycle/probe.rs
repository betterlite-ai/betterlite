use std::vec::IntoIter;

use async_trait::async_trait;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_models::decisions::Json;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::resolution::ResolutionContext;
use betterlite_server::judge_targets::call::JudgeCall;
use betterlite_server::judge_targets::error::JudgeDispatchError;
use betterlite_server::judge_targets::sdk::JudgeSdk;
use betterlite_server::model_visibility::testing::Snapshot;
use betterlite_server::plugin_routes::routes::Identity;
use betterlite_server::shadow_eval_lifecycle::access::require_writer;
use betterlite_server::shadow_eval_lifecycle::body::decode_shadow_body;
use betterlite_server::shadow_eval_lifecycle::error::LifecycleError;
use betterlite_server::shadow_eval_lifecycle::start::{StartCall, StartDeps, start_shadow_eval};
use betterlite_server::shadow_eval_lifecycle::stop::stop_shadow_eval_job;
use betterlite_server::shadow_eval_validation::request::StartShadowEvalRequest;
use betterlite_server::shadow_eval_validation::router_view::{
    AnthropicCredentials, ShadowEvalRouter,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::forms::{StartStep, Step, StopStep};
use super::output::{JobView, StepOutcome};
use super::store::ProbeStore;
use super::{body_bytes, failed, identity_of, parse_outcome, unconsulted};
use crate::router_probes::error::OperationError;

pub(super) struct Probe<'probe> {
    pub(super) store: &'probe ProbeStore,
    pub(super) fixture: &'probe Snapshot,
    pub(super) settings: &'probe AnthropicSettings,
    pub(super) cache: &'probe ProviderCache,
    pub(super) resolution: &'probe ResolutionContext<'probe>,
    pub(super) now: DateTime<Utc>,
    pub(super) minted: IntoIter<Uuid>,
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

impl Probe<'_> {
    pub(super) async fn step(&mut self, step: &Step) -> Result<StepOutcome, OperationError> {
        match step {
            Step::Parse(body) => parse_outcome(body),
            Step::Start(action) => self.start(action).await,
            Step::Stop(action) => Ok(self.stop(action).await),
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
        Ok(self.run_start(store, &identity, &request).await)
    }

    async fn run_start(
        &mut self,
        store: &ProbeStore,
        identity: &Identity,
        request: &StartShadowEvalRequest,
    ) -> StepOutcome {
        let provider = |_model: &str| -> Option<String> { None };
        let sdk = NoSdk;
        let deps = StartDeps {
            router: Some(ShadowEvalRouter {
                catalog: &self.fixture.catalog,
                model_group_alias: &self.fixture.aliases,
                routing_groups: &self.fixture.groups,
                inferred_provider: &provider,
            }),
            sdk: &sdk,
            cache: self.cache,
            resolution: self.resolution,
            credentials: AnthropicCredentials {
                anthropic_key: None,
                api_key: None,
                settings: self.settings,
                secret_state: unconsulted(),
            },
        };
        let mut call = StartCall {
            caller: identity.user_id.as_deref(),
            now: self.now,
            ids: &mut self.minted,
        };
        match start_shadow_eval(store, deps, request, &mut call).await {
            Ok(response) => StepOutcome::Succeeded {
                status: 201,
                job: Box::new(JobView::of(response)),
            },
            Err(error) => failed(&error),
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
                job: Box::new(JobView::of(response)),
            },
            Err(error) => failed(&error),
        }
    }
}
