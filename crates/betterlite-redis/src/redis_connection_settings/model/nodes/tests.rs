#![cfg(test)]

use super::{ClusterNodeConfig, SentinelNodesInput, StartupNodesInput};

#[test]
fn startup_nodes_text_is_falsy_when_empty() {
    assert!(!StartupNodesInput::Text(String::new()).is_truthy());
    assert!(StartupNodesInput::Text("[]".to_owned()).is_truthy());
}

#[test]
fn startup_nodes_list_is_falsy_when_empty() {
    assert!(!StartupNodesInput::Nodes(Vec::new()).is_truthy());
    assert!(
        StartupNodesInput::Nodes(vec![ClusterNodeConfig {
            host: "a".to_owned(),
            port: 7000,
        }])
        .is_truthy()
    );
}

#[test]
fn sentinel_nodes_list_is_falsy_when_empty() {
    assert!(!SentinelNodesInput::Nodes(Vec::new()).is_truthy());
    assert!(SentinelNodesInput::Nodes(vec![("a".to_owned(), 26379)]).is_truthy());
}
