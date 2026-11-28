#![cfg(test)]

use std::collections::BTreeMap;

use super::user_agent;

#[test]
fn session_user_agent_reads_the_environment() {
    assert!(user_agent(&BTreeMap::new()).starts_with("betterlite/"));
    let environment =
        BTreeMap::from([(String::from("LITELLM_USER_AGENT"), String::from("agent-x"))]);
    assert_eq!(user_agent(&environment), "agent-x");
}
