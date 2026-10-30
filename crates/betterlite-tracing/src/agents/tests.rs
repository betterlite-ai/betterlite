#![cfg(test)]

use betterlite_logging::trace_types::{Span, SpanStatus, SpanType};
use rust_decimal::Decimal;

use super::agent_nodes;

fn span(
    span_id: &str,
    parent_span_id: Option<&str>,
    name: &str,
    span_type: SpanType,
    agent: &str,
) -> Span {
    Span {
        span_id: span_id.to_owned(),
        parent_span_id: parent_span_id.map(str::to_owned),
        name: name.to_owned(),
        span_type,
        agent: agent.to_owned(),
        start_offset_ms: Decimal::ZERO,
        duration_ms: Decimal::ONE,
        status: SpanStatus::Ok,
        error: None,
        error_truncated: false,
        input_preview: String::new(),
        model: None,
        input_tokens: 0,
        output_tokens: 0,
        litellm_request_id: None,
        spend: None,
    }
}

#[test]
fn one_node_per_distinct_agent_name_with_invocation_and_call_counts() {
    let spans = vec![
        span("a1", None, "researcher", SpanType::Agent, "researcher"),
        span("a2", None, "researcher", SpanType::Agent, "researcher"),
        span(
            "l1",
            Some("a1"),
            "chat completion",
            SpanType::Llm,
            "researcher",
        ),
        span("t1", Some("a1"), "search", SpanType::Tool, "researcher"),
    ];
    let nodes = agent_nodes(&spans);
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].name, "researcher");
    assert_eq!(nodes[0].invocations, 2);
    assert_eq!(nodes[0].llm_calls, 1);
    assert_eq!(nodes[0].tool_calls, 1);
}

#[test]
fn stops_the_parent_walk_when_there_is_no_parent() {
    let spans = vec![span(
        "a1",
        None,
        "researcher",
        SpanType::Agent,
        "researcher",
    )];
    let nodes = agent_nodes(&spans);
    assert_eq!(nodes[0].parent_agent, None);
}

#[test]
fn stops_the_parent_walk_when_the_parent_is_missing_from_the_spans() {
    let spans = vec![span(
        "a1",
        Some("ghost"),
        "researcher",
        SpanType::Agent,
        "researcher",
    )];
    let nodes = agent_nodes(&spans);
    assert_eq!(nodes[0].parent_agent, None);
}

#[test]
fn stops_the_parent_walk_on_a_self_loop() {
    let spans = vec![span(
        "a1",
        Some("a1"),
        "researcher",
        SpanType::Agent,
        "researcher",
    )];
    let nodes = agent_nodes(&spans);
    assert_eq!(nodes[0].parent_agent, None);
}

#[test]
fn climbs_past_non_agent_ancestors_to_find_a_differently_named_agent() {
    let spans = vec![
        span("outer", None, "outer", SpanType::Agent, "outer"),
        span("tool1", Some("outer"), "tool", SpanType::Tool, "outer"),
        span("inner", Some("tool1"), "inner", SpanType::Agent, "inner"),
    ];
    let nodes = agent_nodes(&spans);
    let inner = nodes
        .iter()
        .find(|node| node.name == "inner")
        .expect("inner node");
    assert_eq!(inner.parent_agent, Some("outer".to_owned()));
}

#[test]
fn skips_same_named_ancestors_instead_of_stopping() {
    // A same-named ancestor is skipped rather than treated as a stop in the parent walk.
    let spans = vec![
        span(
            "inner",
            Some("outer"),
            "researcher",
            SpanType::Agent,
            "researcher",
        ),
        span(
            "orchestrator",
            None,
            "orchestrator",
            SpanType::Agent,
            "orchestrator",
        ),
        span(
            "outer",
            Some("orchestrator"),
            "researcher",
            SpanType::Agent,
            "researcher",
        ),
    ];
    let nodes = agent_nodes(&spans);
    let researcher = nodes
        .iter()
        .find(|node| node.name == "researcher")
        .expect("researcher node");
    assert_eq!(researcher.parent_agent, Some("orchestrator".to_owned()));
}

#[test]
fn excludes_agents_with_zero_invocations() {
    let spans = vec![span("l1", None, "llm", SpanType::Llm, "ghost")];
    let nodes = agent_nodes(&spans);
    assert!(nodes.is_empty());
}

#[test]
fn associates_agent_spend_only_when_every_llm_request_is_recorded() {
    let mut first = span("l1", None, "chat", SpanType::Llm, "researcher");
    first.litellm_request_id = Some("req-1".to_owned());
    first.spend = Some(Decimal::ONE);
    let mut second = span("l2", None, "chat", SpanType::Llm, "researcher");
    second.litellm_request_id = Some("req-2".to_owned());
    second.spend = Some(Decimal::TWO);
    let agent = span("a1", None, "researcher", SpanType::Agent, "researcher");
    let nodes = agent_nodes(&[agent.clone(), first.clone(), second.clone()]);
    assert_eq!(nodes[0].spend, Some(Decimal::new(3, 0)));

    second.spend = None;
    let partial_nodes = agent_nodes(&[agent, first, second]);
    assert_eq!(partial_nodes[0].spend, None);
}
