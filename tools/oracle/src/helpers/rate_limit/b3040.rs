use std::collections::BTreeMap;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::request_quota_view::ServerOnlyFields;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::request_destination::RequestData;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_rate_limit::identity_descriptors::agent_descriptor::AgentDescriptorBuilder;
use betterlite_rate_limit::identity_descriptors::agent_registry::AgentPolicyRegistry;
use betterlite_rate_limit::limiter_records::{
    ModelGroupResolver, TagRateLimit, TagRateLimitResolver,
};
use betterlite_rate_limit::model_descriptors::ModelDescriptorBuilder;
use betterlite_rate_limit::request_scopes::RequestScopeBuilder;
use betterlite_rate_limit::request_scopes::deployment_health::{
    DeploymentHealthError, DeploymentHealthPolicy, DeploymentHealthSource, DeploymentRef,
};
use betterlite_rate_limit::request_scopes::key_scopes::{
    ScopeInputs, create_rate_limit_descriptors,
};
use betterlite_rate_limit::request_scopes::limit_policy::{
    get_enforced_limit, should_enforce_rate_limit,
};
// ast-grep-ignore: no-long-names
use betterlite_rate_limit::request_scopes::request_assembly::build_request_rate_limit_descriptors;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

struct GroupMap(BTreeMap<String, String>);

impl ModelGroupResolver for GroupMap {
    fn resolve(&self, model: &str) -> Option<String> {
        self.0.get(model).cloned()
    }
}

struct FixedRegistry(BTreeMap<String, AgentResponse>);

impl AgentPolicyRegistry for FixedRegistry {
    fn agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.0.get(agent_id).cloned()
    }
}

struct TagLimits(BTreeMap<String, TagRateLimit>);

#[async_trait::async_trait]
impl TagRateLimitResolver for TagLimits {
    async fn resolve(&self, _tag_names: &[String]) -> BTreeMap<String, TagRateLimit> {
        self.0.clone()
    }
}

struct NoRouter;

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

#[derive(Debug, Deserialize)]
pub(crate) struct PolicyCase {
    #[serde(default)]
    rpm_limit_type: Option<String>,
    #[serde(default)]
    tpm_limit_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnforceCase {
    #[serde(default)]
    limit_type: Option<String>,
    model_has_failures: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EnforcedLimitCase {
    #[serde(default)]
    limit_value: Option<i64>,
    #[serde(default)]
    limit_type: Option<String>,
    model_has_failures: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TagLimitRow {
    tag_name: String,
    #[serde(default)]
    rpm_limit: Option<i64>,
    #[serde(default)]
    tpm_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DescriptorCase {
    #[serde(default)]
    auth: BTreeMap<String, JsonData>,
    #[serde(default)]
    server: ServerOnlyFields,
    #[serde(default)]
    data: RequestData,
    #[serde(default)]
    rpm_limit_type: Option<String>,
    #[serde(default)]
    tpm_limit_type: Option<String>,
    #[serde(default)]
    model_has_failures: bool,
    #[serde(default)]
    call_type: Option<String>,
    #[serde(default)]
    groups: BTreeMap<String, String>,
    #[serde(default)]
    registry: Vec<AgentResponse>,
    #[serde(default)]
    tag_limits: Vec<TagLimitRow>,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    DynamicFlag(Box<PolicyCase>),
    ShouldEnforce(Box<EnforceCase>),
    EnforcedLimit(Box<EnforcedLimitCase>),
    Create(Box<DescriptorCase>),
    Assemble(Box<DescriptorCase>),
}

fn auth_of(case: &DescriptorCase) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut auth =
        UserAPIKeyAuth::construct(AuthInput::Dict(case.auth.clone())).or_value_error()?;
    auth.attach_server_only_fields(case.server.clone());
    Ok(auth)
}

fn registry_of(entries: &[AgentResponse]) -> FixedRegistry {
    let by_id = entries
        .iter()
        .map(|agent| (agent.agent_id.clone(), agent.clone()))
        .collect();
    FixedRegistry(by_id)
}

fn tags_of(rows: &[TagLimitRow]) -> TagLimits {
    let limits = rows
        .iter()
        .map(|row| {
            let limit = TagRateLimit {
                rpm_limit: row.rpm_limit,
                tpm_limit: row.tpm_limit,
            };
            (row.tag_name.clone(), limit)
        })
        .collect();
    TagLimits(limits)
}

fn descriptor_builder<'deps>(
    case: &DescriptorCase,
    agents: &'deps FixedRegistry,
    health: &'deps NoRouter,
    tags: &'deps TagLimits,
) -> RequestScopeBuilder<'deps> {
    RequestScopeBuilder {
        window_size: case.window_size,
        model: ModelDescriptorBuilder {
            window_size: case.window_size,
            resolver: Box::new(GroupMap(case.groups.clone())),
        },
        agents: AgentDescriptorBuilder {
            window_size: case.window_size,
            registry: agents,
        },
        health: DeploymentHealthPolicy {
            source: health,
            error_threshold_per_minute: 1,
        },
        tag_resolver: tags,
    }
}

#[derive(Clone, Copy)]
enum DescriptorFunction {
    Create,
    Assemble,
}

fn descriptor_answer(case: &DescriptorCase, function: DescriptorFunction) -> HelperResult {
    let auth = auth_of(case)?;
    let registry = registry_of(&case.registry);
    let tags = tags_of(&case.tag_limits);
    let health = NoRouter;
    let builder = descriptor_builder(case, &registry, &health, &tags);
    let descriptors = match function {
        DescriptorFunction::Create => create_rate_limit_descriptors(
            &builder,
            ScopeInputs {
                auth: &auth,
                data: &case.data,
                rpm_limit_type: case.rpm_limit_type.as_deref(),
                tpm_limit_type: case.tpm_limit_type.as_deref(),
                model_has_failures: case.model_has_failures,
                call_type: case.call_type.as_deref(),
            },
        )
        .or_value_error()?,
        DescriptorFunction::Assemble => {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .or_value_error()?;
            // ast-grep-ignore: no-long-names
            let assembly = build_request_rate_limit_descriptors(
                &builder,
                &auth,
                &case.data,
                case.call_type.as_deref(),
            );
            runtime.block_on(assembly).or_value_error()?
        }
    };
    Ok(json!({"descriptors": descriptors}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::DynamicFlag(case) => {
                let dynamic = case.rpm_limit_type.as_deref() == Some("dynamic")
                    || case.tpm_limit_type.as_deref() == Some("dynamic");
                Ok(json!({"dynamic": dynamic}))
            }
            Input::ShouldEnforce(case) => {
                let enforce =
                    should_enforce_rate_limit(case.limit_type.as_deref(), case.model_has_failures);
                Ok(json!({"enforce": enforce}))
            }
            Input::EnforcedLimit(case) => {
                let limit = get_enforced_limit(
                    case.limit_value,
                    case.limit_type.as_deref(),
                    case.model_has_failures,
                );
                Ok(json!({"limit": limit}))
            }
            Input::Create(case) => descriptor_answer(case, DescriptorFunction::Create),
            Input::Assemble(case) => descriptor_answer(case, DescriptorFunction::Assemble),
        }
    }
}
