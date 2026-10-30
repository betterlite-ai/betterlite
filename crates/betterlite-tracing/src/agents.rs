use std::collections::HashMap;

use betterlite_logging::trace_types::{AgentNode, Span, SpanType};
use indexmap::IndexMap;
use rust_decimal::Decimal;

mod tests;

fn parent_agent_of(span: &Span, by_id: &HashMap<&str, &Span>) -> Option<String> {
    let mut parent_id = span.parent_span_id.clone();
    for _ in 0..by_id.len() {
        let current = parent_id.as_deref()?;
        if current == span.span_id {
            return None;
        }
        let parent = *by_id.get(current)?;
        if parent.span_type == SpanType::Agent && parent.name != span.name {
            return Some(parent.name.clone());
        }
        parent_id = parent.parent_span_id.clone();
    }
    None
}

fn agent_spend(spans: &[Span], agent_name: &str) -> Option<Decimal> {
    let mut by_request: IndexMap<&str, Option<Decimal>> = IndexMap::new();
    for span in spans {
        if span.span_type != SpanType::Llm || span.agent != agent_name {
            continue;
        }
        let Some(request_id) = span.litellm_request_id.as_deref() else {
            continue;
        };
        by_request.insert(request_id, span.spend);
    }
    if by_request.is_empty() {
        return None;
    }
    let mut total = Decimal::ZERO;
    for recorded in by_request.values() {
        let cost = (*recorded)?;
        total = total.checked_add(cost)?;
    }
    Some(total)
}

fn accumulate_invocation(
    nodes: &mut IndexMap<String, AgentNode>,
    by_id: &HashMap<&str, &Span>,
    span: &Span,
) {
    let parent_agent = parent_agent_of(span, by_id);
    let node = nodes.entry(span.name.clone()).or_insert_with(|| AgentNode {
        name: span.name.clone(),
        parent_agent,
        invocations: 0,
        llm_calls: 0,
        tool_calls: 0,
        duration_ms: Decimal::ZERO,
        spend: None,
    });
    node.invocations = node.invocations.saturating_add(1);
    node.duration_ms = node
        .duration_ms
        .checked_add(span.duration_ms)
        .unwrap_or(node.duration_ms);
}

fn accumulate_calls(nodes: &mut IndexMap<String, AgentNode>, span: &Span) {
    let Some(node) = nodes.get_mut(&span.agent) else {
        return;
    };
    match span.span_type {
        SpanType::Llm => node.llm_calls = node.llm_calls.saturating_add(1),
        SpanType::Tool => node.tool_calls = node.tool_calls.saturating_add(1),
        SpanType::Agent | SpanType::Chain | SpanType::Framework => {}
    }
}

#[must_use]
pub fn agent_nodes(spans: &[Span]) -> Vec<AgentNode> {
    let by_id: HashMap<&str, &Span> = spans
        .iter()
        .map(|span| (span.span_id.as_str(), span))
        .collect();
    let mut nodes: IndexMap<String, AgentNode> = IndexMap::new();
    for span in spans {
        if span.span_type == SpanType::Agent {
            accumulate_invocation(&mut nodes, &by_id, span);
        }
    }
    for span in spans {
        accumulate_calls(&mut nodes, span);
    }
    for node in nodes.values_mut() {
        node.spend = agent_spend(spans, &node.name);
    }
    nodes.into_values().collect()
}
