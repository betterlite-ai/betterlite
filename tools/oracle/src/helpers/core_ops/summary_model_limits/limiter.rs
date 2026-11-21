use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_core::api::summary_model_limits::error::LimiterFailure;
use betterlite_core::api::summary_model_limits::rate::{
    BaseDescriptorRequest, DescriptorScope, LimiterCapability, ProxyHooks, ReadOnlyCheck,
    SummaryDescriptor, SummaryRateLimiter,
};
use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum HookMode {
    Limiter,
    #[serde(rename = "none")]
    Unregistered,
    Raises,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Create,
    Team,
    Project,
    Organization,
    Check,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FailureKind {
    Unverifiable,
    Offline,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptedFailure {
    stage: Stage,
    kind: FailureKind,
    detail: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RateScript {
    hook: HookMode,
    missing: Vec<LimiterCapability>,
    base: Vec<SummaryDescriptor>,
    team: Vec<SummaryDescriptor>,
    project: Vec<SummaryDescriptor>,
    organization: Vec<SummaryDescriptor>,
    overall_code: String,
    failure: Option<ScriptedFailure>,
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct Call {
    call: &'static str,
    hook: Option<String>,
    model: Option<String>,
    token: Option<String>,
    rpm_limit_type: Option<JsonData>,
    tpm_limit_type: Option<JsonData>,
    model_has_failures: Option<bool>,
    descriptors: Option<Vec<SummaryDescriptor>>,
    parent_otel_span: Option<&'static str>,
    read_only: Option<bool>,
}

#[derive(Deserialize)]
struct TokenField {
    token: Option<String>,
}

fn blank(call: &'static str, principal: Option<&UserAPIKeyAuth>) -> Call {
    let token = principal
        .and_then(|auth| serde_json::to_value(auth).ok())
        .and_then(|fields| serde_json::from_value::<TokenField>(fields).ok())
        .and_then(|field| field.token);
    Call {
        call,
        hook: None,
        model: None,
        token,
        rpm_limit_type: None,
        tpm_limit_type: None,
        model_has_failures: None,
        descriptors: None,
        parent_otel_span: None,
        read_only: None,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct ScriptedError(String);

pub(super) struct ScriptedLimiter<'script> {
    pub(super) script: &'script RateScript,
    pub(super) journal: &'script Mutex<Vec<Call>>,
    pub(super) retained: Mutex<Option<Vec<SummaryDescriptor>>>,
}

impl ScriptedLimiter<'_> {
    fn record(&self, stage: Stage, call: Call) -> Result<(), LimiterFailure<ScriptedError>> {
        self.journal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        let Some(failure) = self
            .script
            .failure
            .as_ref()
            .filter(|failure| failure.stage == stage)
        else {
            return Ok(());
        };
        let detail = failure.detail.clone();
        match failure.kind {
            FailureKind::Unverifiable => Err(LimiterFailure::Unverifiable { detail }),
            FailureKind::Offline => Err(LimiterFailure::Other(ScriptedError(detail))),
        }
    }
}

impl SummaryRateLimiter for ScriptedLimiter<'_> {
    type Failure = ScriptedError;

    fn supports(&self, capability: LimiterCapability) -> bool {
        !self.script.missing.contains(&capability)
    }

    fn create_rate_limit_descriptors(
        &self,
        request: &BaseDescriptorRequest<'_>,
    ) -> Result<Vec<SummaryDescriptor>, LimiterFailure<ScriptedError>> {
        let mut call = blank("create", Some(request.principal));
        call.model = Some(request.model.to_owned());
        call.rpm_limit_type = request.rpm_limit_type.cloned();
        call.tpm_limit_type = request.tpm_limit_type.cloned();
        call.model_has_failures = Some(request.model_has_failures);
        self.record(Stage::Create, call)?;
        *self.retained.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(self.script.base.clone());
        Ok(self.script.base.clone())
    }

    // LiteLLM's limiter keeps the base list it built, so the helper reports it after the call
    fn add_scope_descriptors(
        &self,
        scope: DescriptorScope,
        principal: &UserAPIKeyAuth,
        requested_model: &str,
        descriptors: &mut Vec<SummaryDescriptor>,
    ) -> Result<(), LimiterFailure<ScriptedError>> {
        let (name, stage, added) = match scope {
            DescriptorScope::Team => ("team", Stage::Team, &self.script.team),
            DescriptorScope::Project => ("project", Stage::Project, &self.script.project),
            DescriptorScope::Organization => (
                "organization",
                Stage::Organization,
                &self.script.organization,
            ),
        };
        let in_place = scope != DescriptorScope::Organization;
        let mut call = blank(name, Some(principal));
        call.model = Some(requested_model.to_owned());
        call.descriptors = in_place.then(|| descriptors.clone());
        self.record(stage, call)?;
        descriptors.extend(added.iter().cloned());
        if in_place {
            *self.retained.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(descriptors.clone());
        }
        Ok(())
    }

    fn should_rate_limit(
        &self,
        check: &ReadOnlyCheck<'_>,
    ) -> impl Future<Output = Result<Option<String>, LimiterFailure<ScriptedError>>> + Send {
        let mut call = blank("check", None);
        call.descriptors = Some(check.descriptors.to_vec());
        call.parent_otel_span = check.parent_otel_span.map(|_span| "set");
        call.read_only = Some(check.read_only);
        let checked = self
            .record(Stage::Check, call)
            .map(|()| Some(self.script.overall_code.clone()));
        ready(checked)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("hooks offline")]
pub(super) struct HooksOffline;

pub(super) struct ScriptedHooks<'script> {
    pub(super) script: &'script RateScript,
    pub(super) journal: &'script Mutex<Vec<Call>>,
    pub(super) limiter: ScriptedLimiter<'script>,
}

impl<'script> ProxyHooks for ScriptedHooks<'script> {
    type Limiter = ScriptedLimiter<'script>;
    type Error = HooksOffline;

    fn get_proxy_hook(
        &self,
        hook: &str,
    ) -> Result<Option<&ScriptedLimiter<'script>>, HooksOffline> {
        let mut call = blank("get_proxy_hook", None);
        call.hook = Some(hook.to_owned());
        self.journal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match self.script.hook {
            HookMode::Limiter => Ok(Some(&self.limiter)),
            HookMode::Unregistered => Ok(None),
            HookMode::Raises => Err(HooksOffline),
        }
    }
}
