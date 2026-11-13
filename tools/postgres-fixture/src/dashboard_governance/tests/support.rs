use std::collections::BTreeMap;

use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::Number;

use crate::dashboard::payload::JsonPayload;
use crate::dashboard::{DashboardSeed, SpendLogSeed};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::DashboardGovernanceSeed;
use crate::dashboard_governance::model::{
    GuardrailIndexRow, GuardrailMetricRow, InputPolicy, OutputPolicy, ToolIndexRow, ToolRow,
    UsageUnitRow,
};
use crate::error::FixtureError;
use crate::finops::FinOpsSeed;
use crate::finops::model::KeyMetadataRow;
use crate::seed::{FixtureSeed, KeyRow, Membership, TeamRow, UserRow};

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn user(user_id: &str) -> UserRow {
    UserRow {
        user_id: user_id.to_owned(),
        user_email: None,
    }
}

fn key(token: &str, user_id: &str, team_id: Option<&str>) -> KeyRow {
    KeyRow {
        token: secret(token),
        user_id: user_id.to_owned(),
        team_id: team_id.map(ToOwned::to_owned),
        role: None,
    }
}

fn team(team_id: &str) -> TeamRow {
    TeamRow {
        team_id: team_id.to_owned(),
        team_alias: None,
    }
}

fn membership(user_id: &str, team_id: &str) -> Membership {
    Membership {
        user_id: user_id.to_owned(),
        team_id: team_id.to_owned(),
    }
}

fn spend_log(request_id: &str, key_index: usize) -> SpendLogSeed {
    SpendLogSeed {
        request_id: request_id.to_owned(),
        call_type: "acompletion".to_owned(),
        start_time: "2026-01-04T10:00:00.000".to_owned(),
        end_time: "2026-01-04T10:00:00.500".to_owned(),
        key_index,
        completion_start_time: None,
        request_duration_ms: None,
        model: None,
        model_id: None,
        model_group: None,
        custom_llm_provider: None,
        user: None,
        team_id: None,
        end_user: None,
        session_id: None,
        status: None,
        cache_hit: None,
        litellm_call_id: None,
        spend: Decimal::ZERO,
        total_tokens: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        metadata: None,
        request_tags: None,
        messages: None,
        response: None,
        proxy_server_request: None,
    }
}

fn number(value: f64) -> JsonPayload {
    JsonPayload::Number(Number::from_f64(value).unwrap())
}

fn guarded_text(value: &str) -> JsonPayload {
    JsonPayload::Text(value.to_owned())
}

fn guardrail_information() -> JsonPayload {
    let response = JsonPayload::Object(BTreeMap::from([(
        "action".to_owned(),
        guarded_text("BLOCKED"),
    )]));
    JsonPayload::Array(vec![JsonPayload::Object(BTreeMap::from([
        ("guardrail_id".to_owned(), guarded_text("gr-pii")),
        ("guardrail_name".to_owned(), guarded_text("pii-guard")),
        ("guardrail_provider".to_owned(), guarded_text("bedrock")),
        ("guardrail_mode".to_owned(), guarded_text("pre_call")),
        (
            "guardrail_status".to_owned(),
            guarded_text("guardrail_intervened"),
        ),
        ("guardrail_response".to_owned(), response),
        ("duration".to_owned(), number(0.25)),
        ("confidence_score".to_owned(), number(0.97)),
        ("risk_score".to_owned(), number(8.5)),
    ]))])
}

fn blocked_log() -> SpendLogSeed {
    let mut log = spend_log("req-blocked", 0);
    log.status = Some("failure".to_owned());
    log.model = Some("openai/gpt-5".to_owned());
    log.metadata = Some(JsonPayload::Object(BTreeMap::from([(
        "guardrail_information".to_owned(),
        guardrail_information(),
    )])));
    log.messages = Some(JsonPayload::Array(vec![JsonPayload::Object(
        BTreeMap::from([
            ("role".to_owned(), guarded_text("user")),
            ("content".to_owned(), guarded_text("hello")),
        ]),
    )]));
    log.response = Some(JsonPayload::Object(BTreeMap::from([(
        "id".to_owned(),
        guarded_text("chatcmpl-blocked"),
    )])));
    log.proxy_server_request = Some(JsonPayload::Object(BTreeMap::from([(
        "url".to_owned(),
        guarded_text("http://localhost/v1/chat/completions"),
    )])));
    log
}

fn tool_log() -> SpendLogSeed {
    let mut log = spend_log("req-tools", 1);
    log.start_time = "2026-01-04T11:00:00.000".to_owned();
    log.end_time = "2026-01-04T11:00:00.400".to_owned();
    log.status = Some("success".to_owned());
    log
}

pub(super) fn sample_dashboard() -> DashboardSeed {
    DashboardSeed {
        logs: vec![blocked_log(), tool_log()],
    }
}

pub(super) fn empty_dashboard() -> DashboardSeed {
    DashboardSeed { logs: Vec::new() }
}

fn sample_metrics() -> Vec<GuardrailMetricRow> {
    vec![
        GuardrailMetricRow {
            guardrail_id: "gr-pii".to_owned(),
            date: "2026-01-04".to_owned(),
            requests_evaluated: 10,
            passed_count: 7,
            blocked_count: 2,
            flagged_count: 1,
            avg_score: Some(Decimal::new(875, 3)),
            avg_latency_ms: Some(Decimal::new(125, 1)),
        },
        GuardrailMetricRow {
            guardrail_id: "gr-config".to_owned(),
            date: "2026-01-04".to_owned(),
            requests_evaluated: 3,
            passed_count: 3,
            blocked_count: 0,
            flagged_count: 0,
            avg_score: None,
            avg_latency_ms: None,
        },
    ]
}

fn sample_usage_units() -> Vec<UsageUnitRow> {
    let mut priced = usage_unit("gr-pii", "contentPolicyUnits");
    priced.units = 5;
    priced.cost = Some(Decimal::new(25, 2));
    let mut unpriced = usage_unit("gr-pii", "textUnits");
    unpriced.team_id = String::new();
    unpriced.key_index = None;
    unpriced.units = 3;
    unpriced.untracked_units = 3;
    vec![priced, unpriced]
}

fn sample_guardrail_index() -> Vec<GuardrailIndexRow> {
    let mut guarded = guardrail_index("req-blocked", "gr-pii");
    guarded.policy_id = Some("policy-eu".to_owned());
    vec![guarded, guardrail_index("req-blocked", "gr-config")]
}

fn sample_tools() -> Vec<ToolRow> {
    let mut github = tool("tool-github", "mcp__github/search");
    github.origin = Some("user_defined".to_owned());
    github.input_policy = InputPolicy::Blocked;
    github.output_policy = OutputPolicy::Trusted;
    github.call_count = 42_i32;
    github.key_index = Some(0);
    github.team_id = Some("team-red".to_owned());
    github.key_alias = Some("alice-discovery-key".to_owned());
    github.user_agent = Some("openai-python/1.0".to_owned());
    github.last_used_at = Some("2026-01-04T11:00:01.000".to_owned());
    vec![github, tool("tool-weather", "get_weather")]
}

pub(super) fn sample_governance() -> DashboardGovernanceSeed {
    DashboardGovernanceSeed {
        metrics: sample_metrics(),
        usage_units: sample_usage_units(),
        guardrail_index: sample_guardrail_index(),
        tool_index: vec![ToolIndexRow {
            request_id: "req-tools".to_owned(),
            tool_name: "mcp__github/search".to_owned(),
            start_time: "2026-01-04T11:00:00.000".to_owned(),
        }],
        tools: sample_tools(),
    }
}

fn key_alias_metadata() -> FinOpsSeed {
    FinOpsSeed {
        organizations: Vec::new(),
        key_metadata: vec![KeyMetadataRow {
            token: secret("sk-alice-token"),
            key_alias: Some("alice-live-key".to_owned()),
            organization_id: None,
            team_organization_id: None,
        }],
        daily_usage: Vec::new(),
        config_rows: Vec::new(),
    }
}

pub(super) fn seed_with(
    dashboard: Option<DashboardSeed>,
    governance: Option<DashboardGovernanceSeed>,
) -> FixtureSeed {
    FixtureSeed {
        users: vec![user("alice"), user("bob")],
        keys: vec![
            key("sk-alice-token", "alice", Some("team-red")),
            key("sk-bob-token", "bob", None),
            key("sk-unrelated-token", "bob", None),
        ],
        teams: vec![team("team-red"), team("team-blue")],
        memberships: vec![
            membership("alice", "team-red"),
            membership("alice", "team-blue"),
        ],
        budgets: Vec::new(),
        jwt_mappings: Vec::new(),
        daily_usage: None,
        finops: Some(key_alias_metadata()),
        discovery_skills: None,
        search_tools: None,
        dashboard,
        dashboard_governance: governance,
    }
}

pub(super) fn sample_seed() -> FixtureSeed {
    seed_with(Some(sample_dashboard()), Some(sample_governance()))
}

pub(super) fn with_governance(governance: DashboardGovernanceSeed) -> FixtureSeed {
    seed_with(Some(sample_dashboard()), Some(governance))
}

pub(super) fn governance_without_dashboard() -> FixtureSeed {
    seed_with(None, Some(sample_governance()))
}

pub(super) fn governance_error(seed: &FixtureSeed) -> DashboardGovernanceError {
    let FixtureError::DashboardGovernance(source) = seed.validate().unwrap_err() else {
        panic!("expected a dashboard governance fixture error");
    };
    source
}

fn metric(guardrail_id: &str, date: &str) -> GuardrailMetricRow {
    GuardrailMetricRow {
        guardrail_id: guardrail_id.to_owned(),
        date: date.to_owned(),
        requests_evaluated: 1,
        passed_count: 1,
        blocked_count: 0,
        flagged_count: 0,
        avg_score: None,
        avg_latency_ms: None,
    }
}

fn usage_unit(guardrail_id: &str, usage_unit: &str) -> UsageUnitRow {
    UsageUnitRow {
        guardrail_id: guardrail_id.to_owned(),
        date: "2026-01-04".to_owned(),
        team_id: "team-red".to_owned(),
        key_index: Some(0),
        usage_unit: usage_unit.to_owned(),
        units: 1,
        cost: None,
        untracked_units: 0,
    }
}

fn guardrail_index(request_id: &str, guardrail_id: &str) -> GuardrailIndexRow {
    GuardrailIndexRow {
        request_id: request_id.to_owned(),
        guardrail_id: guardrail_id.to_owned(),
        policy_id: None,
        start_time: "2026-01-04T10:00:00.000".to_owned(),
    }
}

fn tool_index(request_id: &str, tool_name: &str) -> ToolIndexRow {
    ToolIndexRow {
        request_id: request_id.to_owned(),
        tool_name: tool_name.to_owned(),
        start_time: "2026-01-04T11:00:00.000".to_owned(),
    }
}

fn tool(tool_id: &str, tool_name: &str) -> ToolRow {
    ToolRow {
        tool_id: tool_id.to_owned(),
        tool_name: tool_name.to_owned(),
        origin: None,
        input_policy: InputPolicy::Untrusted,
        output_policy: OutputPolicy::Untrusted,
        call_count: 0,
        key_index: None,
        team_id: None,
        key_alias: None,
        user_agent: None,
        last_used_at: None,
    }
}

fn governance_with_metrics(metrics: Vec<GuardrailMetricRow>) -> FixtureSeed {
    let mut governance = sample_governance();
    governance.metrics = metrics;
    with_governance(governance)
}

fn governance_with_usage_units(usage_units: Vec<UsageUnitRow>) -> FixtureSeed {
    let mut governance = sample_governance();
    governance.usage_units = usage_units;
    with_governance(governance)
}

fn governance_with_guardrail_index(index: Vec<GuardrailIndexRow>) -> FixtureSeed {
    let mut governance = sample_governance();
    governance.guardrail_index = index;
    with_governance(governance)
}

fn governance_with_tool_index(index: Vec<ToolIndexRow>) -> FixtureSeed {
    let mut governance = sample_governance();
    governance.tool_index = index;
    with_governance(governance)
}

fn governance_with_tools(tools: Vec<ToolRow>) -> FixtureSeed {
    let mut governance = sample_governance();
    governance.tools = tools;
    with_governance(governance)
}

pub(super) fn metrics_over_limit() -> FixtureSeed {
    governance_with_metrics(
        (0_usize..129)
            .map(|index| metric(&format!("gr-{index}"), "2026-01-04"))
            .collect(),
    )
}

pub(super) fn usage_units_over_limit() -> FixtureSeed {
    governance_with_usage_units(
        (0_usize..129)
            .map(|index| usage_unit("gr-pii", &format!("unit-{index}")))
            .collect(),
    )
}

pub(super) fn guardrail_index_over_limit() -> FixtureSeed {
    governance_with_guardrail_index(
        (0_usize..129)
            .map(|index| guardrail_index("req-blocked", &format!("gr-{index}")))
            .collect(),
    )
}

pub(super) fn tool_index_over_limit() -> FixtureSeed {
    governance_with_tool_index(
        (0_usize..129)
            .map(|index| tool_index("req-tools", &format!("tool-{index}")))
            .collect(),
    )
}

pub(super) fn tools_over_limit() -> FixtureSeed {
    governance_with_tools(
        (0_usize..129)
            .map(|index| tool(&format!("tool-{index}"), &format!("name-{index}")))
            .collect(),
    )
}

pub(super) fn long_metric_identity() -> FixtureSeed {
    governance_with_metrics(vec![metric(&"g".repeat(129), "2026-01-04")])
}

pub(super) fn long_metric_date() -> FixtureSeed {
    governance_with_metrics(vec![metric("gr-pii", &"d".repeat(129))])
}

pub(super) fn long_tool_identity() -> FixtureSeed {
    governance_with_tools(vec![tool(&"t".repeat(129), "get_weather")])
}

pub(super) fn long_guardrail_policy() -> FixtureSeed {
    let mut row = guardrail_index("req-blocked", "gr-pii");
    row.policy_id = Some("p".repeat(129));
    governance_with_guardrail_index(vec![row])
}

pub(super) fn long_usage_unit_name() -> FixtureSeed {
    let mut row = usage_unit("gr-pii", "contentPolicyUnits");
    row.usage_unit = "u".repeat(4097);
    governance_with_usage_units(vec![row])
}

pub(super) fn long_start_time() -> FixtureSeed {
    let mut index = tool_index("req-tools", "get_weather");
    index.start_time = "s".repeat(129);
    governance_with_tool_index(vec![index])
}

pub(super) fn long_origin() -> FixtureSeed {
    let mut origin_tool = tool("tool-origin", "get_weather");
    origin_tool.origin = Some("o".repeat(4097));
    governance_with_tools(vec![origin_tool])
}

pub(super) fn negative_requests() -> FixtureSeed {
    let mut row = metric("gr-pii", "2026-01-04");
    row.requests_evaluated = -1;
    governance_with_metrics(vec![row])
}

pub(super) fn negative_units() -> FixtureSeed {
    let mut row = usage_unit("gr-pii", "contentPolicyUnits");
    row.units = -1;
    governance_with_usage_units(vec![row])
}

pub(super) fn negative_untracked_units() -> FixtureSeed {
    let mut row = usage_unit("gr-pii", "contentPolicyUnits");
    row.untracked_units = -1;
    governance_with_usage_units(vec![row])
}

pub(super) fn negative_call_count() -> FixtureSeed {
    let mut row = tool("tool-github", "mcp__github/search");
    row.call_count = -1_i32;
    governance_with_tools(vec![row])
}

pub(super) fn duplicate_metric() -> FixtureSeed {
    governance_with_metrics(vec![
        metric("gr-pii", "2026-01-04"),
        metric("gr-pii", "2026-01-04"),
    ])
}

pub(super) fn duplicate_usage_unit() -> FixtureSeed {
    governance_with_usage_units(vec![
        usage_unit("gr-pii", "contentPolicyUnits"),
        usage_unit("gr-pii", "contentPolicyUnits"),
    ])
}

pub(super) fn duplicate_tool_id() -> FixtureSeed {
    governance_with_tools(vec![
        tool("tool-same", "first_name"),
        tool("tool-same", "second_name"),
    ])
}

pub(super) fn duplicate_tool_name() -> FixtureSeed {
    governance_with_tools(vec![
        tool("tool-first", "shared_name"),
        tool("tool-second", "shared_name"),
    ])
}

pub(super) fn duplicate_guardrail_index() -> FixtureSeed {
    governance_with_guardrail_index(vec![
        guardrail_index("req-blocked", "gr-pii"),
        guardrail_index("req-blocked", "gr-pii"),
    ])
}

pub(super) fn duplicate_tool_index() -> FixtureSeed {
    governance_with_tool_index(vec![
        tool_index("req-tools", "get_weather"),
        tool_index("req-tools", "get_weather"),
    ])
}

pub(super) fn unknown_tool_key() -> FixtureSeed {
    let mut row = tool("tool-github", "mcp__github/search");
    row.key_index = Some(7);
    governance_with_tools(vec![row])
}

pub(super) fn unknown_usage_key() -> FixtureSeed {
    let mut row = usage_unit("gr-pii", "contentPolicyUnits");
    row.key_index = Some(7);
    governance_with_usage_units(vec![row])
}

pub(super) fn unknown_tool_team() -> FixtureSeed {
    let mut row = tool("tool-github", "mcp__github/search");
    row.team_id = Some("team-missing".to_owned());
    governance_with_tools(vec![row])
}

pub(super) fn unknown_usage_team() -> FixtureSeed {
    let mut row = usage_unit("gr-pii", "contentPolicyUnits");
    row.team_id = "team-missing".to_owned();
    governance_with_usage_units(vec![row])
}

pub(super) fn unknown_guardrail_log() -> FixtureSeed {
    let mut row = guardrail_index("req-blocked", "gr-pii");
    row.request_id = "req-missing".to_owned();
    governance_with_guardrail_index(vec![row])
}

pub(super) fn unknown_tool_log() -> FixtureSeed {
    let mut row = tool_index("req-tools", "get_weather");
    row.request_id = "req-missing".to_owned();
    governance_with_tool_index(vec![row])
}

pub(super) fn failed_insert() -> FixtureSeed {
    let mut row = tool_index("req-tools", "get_weather");
    row.start_time = "not-a-timestamp".to_owned();
    governance_with_tool_index(vec![row])
}

pub(super) fn boundary_governance() -> FixtureSeed {
    let mut governance = sample_governance();
    governance.metrics = (0_usize..128)
        .map(|index| metric(&format!("g{index:0>127}"), &"d".repeat(128)))
        .collect();
    let mut at_limit = tool(&"t".repeat(128), &"n".repeat(128));
    at_limit.origin = Some("o".repeat(4096));
    at_limit.key_alias = Some("a".repeat(4096));
    at_limit.user_agent = Some("u".repeat(4096));
    at_limit.last_used_at = Some("2".repeat(128));
    governance.tools = vec![at_limit];
    with_governance(governance)
}
