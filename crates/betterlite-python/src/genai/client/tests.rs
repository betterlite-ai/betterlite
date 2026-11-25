#![cfg(test)]

use std::collections::BTreeMap;

use super::user_agent;

#[test]
fn user_agent_reads_the_environment_then_the_crate() {
    let environment = BTreeMap::from([("LITELLM_USER_AGENT".to_owned(), "probe/1".to_owned())]);
    assert_eq!(user_agent(&environment), "probe/1");
    assert!(user_agent(&BTreeMap::new()).starts_with("betterlite/"));
}
