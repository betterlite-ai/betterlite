use betterlite_postgres_fixture::dashboard_governance::DashboardGovernanceSeed;

pub(super) fn discovery(seed: &DashboardGovernanceSeed) -> Vec<String> {
    let mut lines = Vec::new();
    for index in &seed.guardrail_index {
        lines.push(format!(
            "guard|{}|{}|{:?}|{}",
            index.request_id, index.guardrail_id, index.policy_id, index.start_time,
        ));
    }
    for index in &seed.tool_index {
        lines.push(format!(
            "tool_index|{}|{}|{}",
            index.request_id, index.tool_name, index.start_time,
        ));
    }
    for tool in &seed.tools {
        lines.push(format!(
            "tool|{}|{}|{:?}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}",
            tool.tool_id,
            tool.tool_name,
            tool.origin,
            tool.input_policy.as_wire(),
            tool.output_policy.as_wire(),
            tool.call_count,
            tool.key_index,
            tool.team_id,
            tool.key_alias,
            tool.user_agent,
            tool.last_used_at,
        ));
    }
    lines
}
