#![cfg(test)]

use std::collections::BTreeMap;
use std::iter;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::kill_switch::{
    AgentKillSwitchAuth, AgentKillSwitchConfig, ApiKeyHeader,
};
use betterlite_core::agents::{AgentKillSwitchMethod, AgentResponse};
use betterlite_redact::structured::SensitiveDataMasker;
use secrecy::SecretString;

use crate::agent_credentials::REDACTED_BY_LITELM;

pub(crate) fn masker() -> SensitiveDataMasker {
    SensitiveDataMasker::with_patterns(Vec::<String>::new(), Vec::<String>::new())
}

pub(crate) fn json(text: &str) -> AgentJson {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn marker() -> AgentJson {
    AgentJson::String(String::from(REDACTED_BY_LITELM))
}

pub(crate) fn nested(levels: usize, value: AgentJson) -> AgentJson {
    let mut current = value;
    for _ in 0..levels {
        current = AgentJson::Object(iter::once((String::from("k"), current)).collect());
    }
    current
}

pub(crate) fn agent(agent_id: &str) -> AgentResponse {
    let text = format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_id}","agent_card_params":{{}}}}"#
    );
    serde_json::from_str(&text).unwrap()
}

pub(crate) fn agent_with_identity(agent_id: &str) -> AgentResponse {
    let text = format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"{agent_id}","agent_card_params":{{}},"identity":{{"agent_id":"{agent_id}","provider":"microsoft_entra","tenant_id":"550e8400-e29b-41d4-a716-446655440000","client_id":"550e8400-e29b-41d4-a716-446655440001","issuer":"https://login.example/v2.0","revision":"1"}}}}"#
    );
    serde_json::from_str(&text).unwrap()
}

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

pub(crate) fn bearer(token: &str) -> AgentKillSwitchAuth {
    AgentKillSwitchAuth::Bearer {
        token: secret(token),
    }
}

pub(crate) fn api_key_auth(header_name: &str, api_key: &str) -> AgentKillSwitchAuth {
    AgentKillSwitchAuth::ApiKey {
        header_name: ApiKeyHeader(String::from(header_name)),
        api_key: secret(api_key),
    }
}

pub(crate) fn basic_auth(username: &str, password: &str) -> AgentKillSwitchAuth {
    AgentKillSwitchAuth::Basic {
        username: String::from(username),
        password: secret(password),
    }
}

pub(crate) fn switch(auth: Option<AgentKillSwitchAuth>) -> AgentKillSwitchConfig {
    AgentKillSwitchConfig {
        url: String::from("https://agent.example/stop"),
        method: AgentKillSwitchMethod::Post,
        headers: BTreeMap::new(),
        query_params: BTreeMap::new(),
        body: None,
        auth,
    }
}
