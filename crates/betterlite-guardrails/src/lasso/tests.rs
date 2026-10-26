#![cfg(test)]

use super::wire::{
    Finding, LassoRequestData, LassoResponse, MessageType, OpenAiFunction, OpenAiTool,
    ToolArguments,
};
use super::{ConversationCache, LassoError, LassoGuardrail, LassoSettings};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Default)]
struct TestCache(Mutex<BTreeMap<String, String>>);

impl ConversationCache for TestCache {
    fn get_cache(&self, key: &str) -> Result<Option<String>, String> {
        let values = self.0.lock().map_err(|error| error.to_string())?;
        Ok(values.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: &str, _ttl_seconds: u64) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|error| error.to_string())?
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }
}

#[test]
fn lasso_guardrail_constructor_requires_api_key_and_defaults_base() {
    assert!(matches!(
        LassoGuardrail::new(LassoSettings::default(), reqwest::Client::default()),
        Err(LassoError::MissingSecrets)
    ));
    let guardrail = LassoGuardrail::new(
        LassoSettings {
            api_key: Some("key".to_owned()),
            api_base: None,
            user_id: None,
            conversation_id: None,
            mask: false,
        },
        reqwest::Client::default(),
    )
    .unwrap();
    assert_eq!(
        guardrail.api_base,
        "https://server.lasso.security/gateway/v3"
    );
}

#[test]
fn lasso_guardrail_config_defaults_masking_off() {
    assert!(!LassoSettings::default().mask);
}

#[test]
fn lasso_guardrail_session_cache_reuses_call_id() {
    let cache = TestCache::default();
    let guardrail = LassoGuardrail::new(
        LassoSettings {
            api_key: Some("key".to_owned()),
            api_base: None,
            user_id: None,
            conversation_id: None,
            mask: false,
        },
        reqwest::Client::default(),
    )
    .unwrap();
    let generated = guardrail.conversation_id(Some("call-1"), &cache);
    assert_eq!(generated, guardrail.conversation_id(Some("call-1"), &cache));
}

#[test]
fn lasso_guardrail_preserves_malformed_tool_arguments() {
    let tool = OpenAiTool {
        function: Some(OpenAiFunction {
            name: Some("lookup".to_owned()),
            description: None,
            parameters: None,
            arguments: Some("smuggled PII".to_owned()),
        }),
    };
    assert!(
        matches!(super::extract_tool_arguments(&tool), Some(ToolArguments::Malformed { arguments }) if arguments == "smuggled PII")
    );
}

#[test]
fn lasso_guardrail_only_blocks_block_actions() {
    let guardrail = LassoGuardrail::new(
        LassoSettings {
            api_key: Some("key".to_owned()),
            api_base: None,
            user_id: None,
            conversation_id: None,
            mask: false,
        },
        reqwest::Client::default(),
    )
    .unwrap();
    let mut findings = BTreeMap::new();
    findings.insert(
        "pii".to_owned(),
        vec![Finding {
            action: Some("AUTO_MASKING".to_owned()),
        }],
    );
    let warning = LassoResponse {
        violations_detected: true,
        findings,
    };
    guardrail.process_response(warning).unwrap();
    let blocked_findings = BTreeMap::from([(
        "jailbreak".to_owned(),
        vec![Finding {
            action: Some("BLOCK".to_owned()),
        }],
    )]);
    let blocked = LassoResponse {
        violations_detected: true,
        findings: blocked_findings,
    };
    assert!(matches!(
        guardrail.process_response(blocked),
        Err(LassoError::Blocked { .. })
    ));
}

#[test]
fn lasso_guardrail_payload_has_session_and_source() {
    let cache = TestCache::default();
    let guardrail = LassoGuardrail::new(
        LassoSettings {
            api_key: Some("key".to_owned()),
            api_base: None,
            user_id: None,
            conversation_id: None,
            mask: false,
        },
        reqwest::Client::default(),
    )
    .unwrap();
    let message =
        RawValue::from_string("{\"role\":\"user\",\"content\":\"hello\"}".to_owned()).unwrap();
    let data = LassoRequestData {
        litellm_call_id: Some("call-1".to_owned()),
        tools: Vec::new(),
    };
    let payload = guardrail.prepare_payload(vec![message], &data, MessageType::Prompt, &cache);
    assert_eq!(
        payload.session_id,
        guardrail.conversation_id(Some("call-1"), &cache)
    );
    assert_eq!(payload.source.source_type, "litellm");
}
