#![cfg(test)]

use super::{ClusterNodeInput, SentinelNodeInput, SentinelNodesInput, StartupNodesInput};
use crate::redis_connection_settings::model::scalars::IntOrText;

#[test]
fn startup_nodes_text_is_falsy_when_empty() {
    assert!(!StartupNodesInput::Text(String::new()).is_truthy());
    assert!(StartupNodesInput::Text("[]".to_owned()).is_truthy());
}

#[test]
fn startup_nodes_list_is_falsy_when_empty() {
    assert!(!StartupNodesInput::Nodes(Vec::new()).is_truthy());
    assert!(
        StartupNodesInput::Nodes(vec![ClusterNodeInput {
            host: "a".to_owned(),
            port: IntOrText::Int(7000),
        }])
        .is_truthy()
    );
}

#[test]
fn sentinel_nodes_list_is_falsy_when_empty() {
    assert!(!SentinelNodesInput::Nodes(Vec::new()).is_truthy());
    assert!(
        SentinelNodesInput::Nodes(vec![SentinelNodeInput::Tuple((
            "a".to_owned(),
            IntOrText::Int(26379),
        ))])
        .is_truthy()
    );
}

#[test]
fn node_ports_accept_quoted_json_numbers() {
    let startup: StartupNodesInput = serde_json::from_str(r#"[{"host":"cache","port":"7000"}]"#)
        .expect("quoted cluster ports are accepted");
    assert!(
        matches!(startup, StartupNodesInput::Nodes(nodes) if nodes.first().is_some_and(|node| matches!(&node.port, IntOrText::Text(port) if port == "7000")))
    );

    let sentinel: SentinelNodesInput = serde_json::from_str(r#"[["sentinel","26379"]]"#)
        .expect("quoted sentinel ports are accepted");
    assert!(
        matches!(sentinel, SentinelNodesInput::Nodes(nodes) if nodes.first().is_some_and(|node| matches!(node, SentinelNodeInput::Tuple((_, IntOrText::Text(port))) if port == "26379")))
    );
}
