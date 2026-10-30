#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use secrecy::ExposeSecret as _;

#[test]
fn cisco_config_loads_secret_and_defaults() {
    let source = "guardrails:\n  - guardrail_name: cisco\n    litellm_params:\n      guardrail: cisco_ai_defense\n";
    let config = ConfigLoader {
        lookup: BTreeMap::from([(
            String::from("CISCO_AI_DEFENSE_API_KEY"),
            String::from("secret"),
        )]),
    }
    .load(source)
    .expect("config");
    let configured = &config.sections.cisco_verdict_contracts.guardrails[0];
    assert_eq!(configured.api_key.expose_secret(), "secret");
    assert_eq!(configured.timeout, 10.0);
    assert_eq!(configured.inspect_path, "/api/v1/inspect/chat");
}
