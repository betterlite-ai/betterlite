use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::request_quota_view::ServerOnlyFields;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::request_destination::RequestData;
use betterlite_db::budget_permission_rows::JsonData;

use crate::identity_descriptors::agent_descriptor::AgentDescriptorBuilder;
use crate::identity_descriptors::agent_registry::AgentPolicyRegistry;
use crate::limiter_records::{ModelGroupResolver, TagRateLimit, TagRateLimitResolver};
use crate::model_descriptors::ModelDescriptorBuilder;
use crate::request_scopes::RequestScopeBuilder;
use crate::request_scopes::deployment_health::{
    DeploymentHealthError, DeploymentHealthPolicy, DeploymentHealthSource, DeploymentRef,
};

pub(crate) fn auth(fields: &str) -> UserAPIKeyAuth {
    let values: BTreeMap<String, JsonData> = serde_json::from_str(fields).expect("auth fields");
    UserAPIKeyAuth::construct(AuthInput::Dict(values)).expect("the auth fixture builds")
}

pub(crate) fn attach(auth: &mut UserAPIKeyAuth, wire: &str) {
    let fields: ServerOnlyFields = serde_json::from_str(wire).expect("server only fields");
    auth.attach_server_only_fields(fields);
}

pub(crate) fn data(wire: &str) -> RequestData {
    serde_json::from_str(wire).expect("request body")
}

pub(crate) fn policy(agent_id: &str, rpm_limit: Option<i64>) -> AgentResponse {
    let limit = match rpm_limit {
        Some(limit) => format!(r#", "rpm_limit": {limit}"#),
        None => String::new(),
    };
    let wire = format!(
        r#"{{"agent_id": "{agent_id}", "agent_name": "{agent_id}", "agent_card_params": {{}}{limit}}}"#
    );
    serde_json::from_str(&wire).expect("the policy fixture decodes")
}

struct GroupMap(BTreeMap<String, String>);

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

pub(crate) struct FixedRegistry(pub(crate) BTreeMap<String, AgentResponse>);

impl AgentPolicyRegistry for FixedRegistry {
    fn agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.0.get(agent_id).cloned()
    }
}

pub(crate) struct NoRouter;

#[async_trait::async_trait]
impl DeploymentHealthSource for NoRouter {
    async fn deployments_for_model(
        &self,
        _model: &str,
    ) -> Result<Option<Vec<DeploymentRef>>, DeploymentHealthError> {
        Ok(None)
    }

    async fn failures_this_minute(
        &self,
        _deployment_id: &str,
    ) -> Result<i64, DeploymentHealthError> {
        Ok(0)
    }
}

pub(crate) struct RecordingHealth {
    failing: bool,
    models: Mutex<Vec<String>>,
}

impl RecordingHealth {
    pub(crate) const fn healthy() -> Self {
        Self {
            failing: false,
            models: Mutex::new(Vec::new()),
        }
    }

    pub(crate) const fn failing() -> Self {
        Self {
            failing: true,
            models: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn models(&self) -> Vec<String> {
        match self.models.lock() {
            Ok(models) => models.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait::async_trait]
impl DeploymentHealthSource for RecordingHealth {
    async fn deployments_for_model(
        &self,
        model: &str,
    ) -> Result<Option<Vec<DeploymentRef>>, DeploymentHealthError> {
        match self.models.lock() {
            Ok(mut models) => models.push(model.to_owned()),
            Err(poisoned) => poisoned.into_inner().push(model.to_owned()),
        }
        Ok(Some(vec![DeploymentRef {
            id: Some("deployment-1".to_owned()),
        }]))
    }

    async fn failures_this_minute(
        &self,
        _deployment_id: &str,
    ) -> Result<i64, DeploymentHealthError> {
        Ok(if self.failing { 2 } else { 0 })
    }
}

pub(crate) struct MapResolver {
    limits: BTreeMap<String, TagRateLimit>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl MapResolver {
    pub(crate) fn new(limits: &[(&str, Option<i64>, Option<i64>)]) -> Self {
        let limits = limits
            .iter()
            .map(|(tag, rpm, tpm)| {
                (
                    (*tag).to_owned(),
                    TagRateLimit {
                        rpm_limit: *rpm,
                        tpm_limit: *tpm,
                    },
                )
            })
            .collect();
        Self {
            limits,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Vec<String>> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[async_trait::async_trait]
impl TagRateLimitResolver for MapResolver {
    async fn resolve(&self, tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        match self.calls.lock() {
            Ok(mut calls) => calls.push(tag_names.to_vec()),
            Err(poisoned) => poisoned.into_inner().push(tag_names.to_vec()),
        }
        self.limits.clone()
    }
}

pub(crate) fn builder<'deps>(
    window_size: i64,
    groups: &[(&str, &str)],
    registry: &'deps dyn AgentPolicyRegistry,
    health: &'deps dyn DeploymentHealthSource,
    tags: &'deps dyn TagRateLimitResolver,
) -> RequestScopeBuilder<'deps> {
    let by_name = groups
        .iter()
        .map(|(requested, group)| ((*requested).to_owned(), (*group).to_owned()))
        .collect();
    RequestScopeBuilder {
        window_size,
        model: ModelDescriptorBuilder {
            window_size,
            resolver: Box::new(GroupMap(by_name)),
        },
        agents: AgentDescriptorBuilder {
            window_size,
            registry,
        },
        health: DeploymentHealthPolicy {
            source: health,
            error_threshold_per_minute: 1,
        },
        tag_resolver: tags,
    }
}
