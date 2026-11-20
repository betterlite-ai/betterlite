use std::collections::BTreeMap;
use std::future::Future;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::auth_cache::keys::CacheKey;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_core::api::request_destination::RequestData;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_rate_limit::identity_descriptors::agent_descriptor::AgentDescriptorBuilder;
use betterlite_rate_limit::identity_descriptors::mcp_per_key::add_mcp_per_key_descriptor;
use betterlite_rate_limit::identity_descriptors::mcp_per_team::add_mcp_per_team_descriptor;
use betterlite_rate_limit::identity_descriptors::tag_create::create_tag_descriptors;
use betterlite_rate_limit::identity_descriptors::tag_descriptor::tag_rate_limit_descriptor;
use betterlite_rate_limit::identity_descriptors::tag_per_key::add_tag_per_key_descriptor;
use betterlite_rate_limit::identity_descriptors::tag_resolution::resolve_tag_rate_limits;
use betterlite_rate_limit::limiter_records::{RateLimitDescriptor, TagRateLimit};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fakes::{FixedRegistry, FixtureRows, MapResolver, TokenCache};

#[derive(Debug, Deserialize)]
pub(crate) struct TagDescriptorCase {
    tag: String,
    #[serde(default)]
    rpm_limit: Option<i64>,
    #[serde(default)]
    tpm_limit: Option<i64>,
    window_size: i64,
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
pub(crate) struct TagCreateCase {
    #[serde(default)]
    data: RequestData,
    #[serde(default)]
    tag_limits: Vec<TagLimitRow>,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
struct TagBudgetCase {
    #[serde(default)]
    rpm_limit: Option<i64>,
    #[serde(default)]
    tpm_limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TagFixture {
    tag_name: String,
    #[serde(default)]
    budget: Option<TagBudgetCase>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TagResolutionCase {
    #[serde(default)]
    tag_names: Vec<String>,
    database: bool,
    #[serde(default)]
    cached: Vec<TagFixture>,
    #[serde(default)]
    rows: Vec<TagFixture>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TagPerKeyCase {
    auth: BTreeMap<String, JsonData>,
    #[serde(default)]
    data: RequestData,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpPerKeyCase {
    auth: BTreeMap<String, JsonData>,
    #[serde(default)]
    mcp_server_name: Option<String>,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpPerTeamCase {
    auth: BTreeMap<String, JsonData>,
    #[serde(default)]
    mcp_server_name: Option<String>,
    #[serde(default)]
    source_team_rpm_limits: Option<BTreeMap<String, BTreeMap<String, i64>>>,
    window_size: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentCase {
    agent_id: String,
    #[serde(default)]
    data: RequestData,
    window_size: i64,
    #[serde(default)]
    policy: Option<AgentResponse>,
    #[serde(default)]
    registry: Vec<AgentResponse>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentResolutionCase {
    #[serde(default)]
    pub(crate) key_agent_id: Option<String>,
    #[serde(default)]
    pub(crate) data: RequestData,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionCase {
    #[serde(default)]
    pub(crate) data: RequestData,
}

fn block_on<Run: Future>(future: Run) -> Result<Run::Output, Box<HelperError>> {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    Ok(runtime.block_on(future))
}

fn tag_tables(fixtures: &[TagFixture]) -> Result<Vec<TagTable>, Box<HelperError>> {
    let mut tables = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        let wire = match &fixture.budget {
            Some(budget) => json!({
                "tag_name": fixture.tag_name,
                "models": [],
                "spend": 0,
                "litellm_budget_table": {
                    "rpm_limit": budget.rpm_limit,
                    "tpm_limit": budget.tpm_limit,
                },
            }),
            None => json!({"tag_name": fixture.tag_name, "models": [], "spend": 0}),
        };
        tables.push(serde_json::from_value(wire).or_value_error()?);
    }
    Ok(tables)
}

pub(super) fn tag_descriptor(case: &TagDescriptorCase) -> HelperResult {
    let limit = TagRateLimit {
        rpm_limit: case.rpm_limit,
        tpm_limit: case.tpm_limit,
    };
    let descriptors = vec![tag_rate_limit_descriptor(
        &case.tag,
        limit,
        case.window_size,
    )];
    Ok(json!({"descriptors": descriptors}))
}

pub(super) fn tag_create(case: &TagCreateCase) -> HelperResult {
    let mut limits = BTreeMap::new();
    for row in &case.tag_limits {
        let limit = TagRateLimit {
            rpm_limit: row.rpm_limit,
            tpm_limit: row.tpm_limit,
        };
        limits.insert(row.tag_name.clone(), limit);
    }
    let resolver = MapResolver::new(limits);
    let descriptors = block_on(create_tag_descriptors(
        &case.data,
        case.window_size,
        &resolver,
    ))?;
    Ok(json!({
        "descriptors": descriptors,
        "resolver_calls": resolver.calls(),
    }))
}

pub(super) fn tag_resolution(case: &TagResolutionCase) -> HelperResult {
    let mut cached = BTreeMap::new();
    for tag in tag_tables(&case.cached)? {
        cached.insert(CacheKey::Tag(&tag.tag_name).render(), tag);
    }
    let cache = TokenCache(cached);
    let rows = FixtureRows(tag_tables(&case.rows)?);
    let outcome = block_on(resolve_tag_rate_limits(
        &case.tag_names,
        case.database,
        &cache,
        &rows,
    ))?;
    let limits = outcome.or_value_error()?;
    let mut view = BTreeMap::new();
    for (name, limit) in limits {
        view.insert(
            name,
            json!({"rpm_limit": limit.rpm_limit, "tpm_limit": limit.tpm_limit}),
        );
    }
    Ok(json!({"limits": view}))
}

pub(super) fn tag_per_key(case: &TagPerKeyCase) -> HelperResult {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(case.auth.clone())).or_value_error()?;
    let mut descriptors = Vec::new();
    add_tag_per_key_descriptor(&auth, &case.data, case.window_size, &mut descriptors);
    Ok(json!({"descriptors": descriptors}))
}

fn mcp_answer<Add>(fields: &BTreeMap<String, JsonData>, add: Add) -> HelperResult
where
    Add: FnOnce(&UserAPIKeyAuth, &mut Vec<RateLimitDescriptor>),
{
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields.clone())).or_value_error()?;
    let mut descriptors = Vec::new();
    add(&auth, &mut descriptors);
    Ok(json!({"descriptors": descriptors}))
}

pub(super) fn mcp_per_key(case: &McpPerKeyCase) -> HelperResult {
    mcp_answer(&case.auth, |auth, descriptors| {
        let server = case.mcp_server_name.as_deref();
        add_mcp_per_key_descriptor(auth, server, case.window_size, descriptors);
    })
}

pub(super) fn mcp_per_team(case: &McpPerTeamCase) -> HelperResult {
    mcp_answer(&case.auth, |auth, descriptors| {
        let server = case.mcp_server_name.as_deref();
        let teams = case.source_team_rpm_limits.as_ref();
        add_mcp_per_team_descriptor(auth, server, teams, case.window_size, descriptors);
    })
}

pub(super) fn agent_descriptors(case: &AgentCase) -> HelperResult {
    let mut by_id = BTreeMap::new();
    for agent in &case.registry {
        by_id.insert(agent.agent_id.clone(), agent.clone());
    }
    let registry = FixedRegistry(by_id);
    let builder = AgentDescriptorBuilder {
        window_size: case.window_size,
        registry: &registry,
    };
    let descriptors =
        builder.create_agent_descriptors(&case.agent_id, &case.data, case.policy.as_ref());
    Ok(json!({"descriptors": descriptors}))
}
