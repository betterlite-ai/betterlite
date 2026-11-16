use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};
use betterlite_provider::a2a_chat::auth::EntraResolver;
use betterlite_provider::a2a_chat::records::A2aReply;
use betterlite_provider::a2a_chat::registry::{
    A2aAgentLookup, RegisteredA2aAgent, RegistryInput, resolve_agent_config,
};
use betterlite_provider::a2a_chat::reply::{ReplyInput, TokenCounter, transform_response};
use betterlite_provider::a2a_chat::request::{
    get_complete_url, map_openai_params, transform_request, validate_environment,
};
use betterlite_provider::a2a_chat::stream::chunk_parser;
use betterlite_provider::ollama_chat::ids::RandomIds;
use betterlite_tokenizer::count::error::CountError;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::script::{
    AgentCase, ProbeCase, RequestCase, ResponseCase, StreamCase, helper_error, runtime,
};
use betterlite_oracle_helper_api::helper::HelperResult;

struct StoredToken;

impl EntraResolver for StoredToken {
    fn token(&self, params: &Object) -> Result<SecretString, CredentialsError> {
        match params.get("azure_ad_token") {
            Some(Json::Text(token)) => Ok(SecretString::from(token.clone())),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => Err(CredentialsError::AgentTokenUnresolved {
                help: "the oracle helper mints no Entra token",
            }),
        }
    }

    fn auth_header(&self, _params: &Object) -> Result<BTreeMap<String, String>, CredentialsError> {
        Err(CredentialsError::AgentTokenUnresolved {
            help: "the oracle helper resolves no Entra header",
        })
    }
}

struct Lookup {
    agents: Vec<(String, RegisteredA2aAgent)>,
}

impl A2aAgentLookup for Lookup {
    fn agent_by_name(&self, name: &str) -> Option<RegisteredA2aAgent> {
        self.agents
            .iter()
            .find(|(agent_name, _agent)| agent_name == name)
            .map(|(_agent_name, agent)| agent.clone())
    }
}

struct NoCounter;

#[async_trait]
impl TokenCounter for NoCounter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(0)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(0)
    }
}

fn prompt() -> Vec<Message> {
    vec![
        Message {
            role: String::from("user"),
            content: Some(Content::Text(String::from("hello there"))),
            name: None,
            provider_specific_fields: Object::new(),
        },
        Message {
            role: String::from("user"),
            content: Some(Content::Text(String::from("second"))),
            name: None,
            provider_specific_fields: Object::new(),
        },
    ]
}

fn build_lookup(agents: &[AgentCase]) -> Lookup {
    let mut found = Vec::with_capacity(agents.len());
    for agent in agents {
        let registered = RegisteredA2aAgent {
            agent_card_params: agent.card.clone(),
            litellm_params: agent.params.clone(),
        };
        found.push((agent.name.clone(), registered));
    }
    Lookup { agents: found }
}

pub(super) fn registry(agents: &[AgentCase], probes: &[ProbeCase]) -> Value {
    let lookup = build_lookup(agents);
    let mut found = Vec::with_capacity(probes.len());
    for probe in probes {
        let mut optional_params = probe.optional_params.clone();
        let input = RegistryInput {
            agent_name: &probe.agent_name,
            api_base: probe.api_base.as_deref(),
            api_key: probe.api_key.as_ref(),
            headers: probe.headers.as_ref(),
        };
        let resolved =
            resolve_agent_config(&input, &mut optional_params, Some(&lookup), &StoredToken);
        found.push(match resolved {
            Ok(filled) => json!({
                "api_base": filled.api_base,
                "api_key": filled.api_key.as_ref().map(ExposeSecret::expose_secret),
                "headers": filled.headers,
                "optional_params": optional_params,
            }),
            Err(failure) => json!({"error": failure.to_string()}),
        });
    }
    json!({"probes": found})
}

pub(super) fn empty_request() -> Value {
    let empty = Object::new();
    match transform_request(&[], &empty, &RandomIds) {
        Ok(_request) => json!({"raised": null}),
        Err(failure) => json!({"raised": failure.to_string()}),
    }
}

fn request_value(case: &RequestCase, messages: &[Message]) -> HelperResult {
    let mut optional_params = case.optional_params.clone();
    map_openai_params(&case.non_default_params, &mut optional_params);
    let headers = validate_environment(&case.headers, case.api_key.as_ref());
    let url = get_complete_url(case.api_base.as_deref())
        .map_err(|failure| helper_error("ValueError", failure.to_string()))?;
    let body = transform_request(messages, &optional_params, &RandomIds)
        .map_err(|failure| helper_error("ValueError", failure.to_string()))?;
    Ok(json!({"url": url, "headers": headers, "body": body}))
}

pub(super) fn requests(cases: &[RequestCase]) -> HelperResult {
    let messages = prompt();
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        found.push(request_value(case, &messages)?);
    }
    Ok(json!({"cases": found}))
}

fn reply_value(reply: &A2aReply) -> Value {
    let Some(choice) = reply.choices.first() else {
        return json!(null);
    };
    json!({
        "id": reply.id,
        "model": reply.model,
        "index": choice.index,
        "finish_reason": choice.finish_reason,
        "role": choice.message.role,
        "content": choice.message.content,
    })
}

pub(super) fn responses(cases: &[ResponseCase]) -> HelperResult {
    let runtime = runtime()?;
    let messages = prompt();
    let headers = BTreeMap::new();
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let input = ReplyInput {
            model: &case.model,
            status: case.status,
            headers: &headers,
            body: case.body.as_bytes(),
            messages: &messages,
        };
        let reply = runtime.block_on(transform_response(&input, &RandomIds, &NoCounter));
        found.push(match reply {
            Ok(parsed) => json!({"reply": reply_value(&parsed)}),
            Err(error) => json!({"error": {"status": error.status_code, "message": error.message}}),
        });
    }
    Ok(json!({"cases": found}))
}

pub(super) fn chunks(cases: &[StreamCase]) -> HelperResult {
    let mut found = Vec::with_capacity(cases.len());
    for case in cases {
        let wire = serde_json::to_vec(&case.chunk)
            .map_err(|failure| helper_error("ValueError", failure.to_string()))?;
        found.push(match chunk_parser(&wire) {
            Ok(parsed) => json!({"chunk": {
                "text": parsed.text,
                "is_finished": parsed.is_finished,
                "finish_reason": parsed.finish_reason,
                "index": parsed.index,
            }}),
            Err(error) => json!({"error": {"status": error.status_code, "message": error.message}}),
        });
    }
    Ok(json!({"chunks": found}))
}
