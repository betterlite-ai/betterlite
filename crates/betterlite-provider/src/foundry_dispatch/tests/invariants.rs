use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::Headers;
use secrecy::ExposeSecret as _;

use super::support::{self, Env, REPLY, chat_call, chat_http, field, run_chat, text_header};
use crate::foundry_dispatch::chat::prepare::Plan;
use crate::foundry_dispatch::chat::{self, Answer};
use crate::foundry_dispatch::fixtures::logger::RecordingLogger;
use crate::operation::{ChatCompletion, OperationError, OperationResult};
use crate::prepared::deployment::Deployment;

const ALPHA_KEY: &str = "key-alpha-0000000000";
const BETA_KEY: &str = "key-beta-1111111111";
const ALPHA_BASE: &str = "https://alpha.services.ai.azure.com/models";
const BETA_BASE: &str = "https://beta.openai.azure.com/models";

// One caller identity: its global key and tenant header, its deployment and its request
struct Party {
    env: Env,
    deployment: Deployment,
    request: ChatRequest,
    model: &'static str,
}

impl Party {
    const fn call<'party>(&'party self, headers: Option<&'party Headers>) -> chat::Call<'party> {
        chat::Call {
            headers,
            ..chat_call(&self.request, &self.deployment, self.model, false)
        }
    }

    fn plan(&self, headers: Option<&Headers>) -> Plan {
        chat::prepare::prepare(&self.call(headers), &self.env.globals())
            .expect("the identity prepares")
    }

    fn run(&self, answer: Answer<'_>) -> (Result<chat::Outcome, OperationError>, RecordingLogger) {
        run_chat(self.call(None), &self.env, answer)
    }
}

fn identity(key: &str, tenant: &str) -> Env {
    Env::new().global_key(key).headers(Headers::from([(
        String::from("x-tenant"),
        tenant.to_owned(),
    )]))
}

fn alpha(deployment_key: Option<&str>) -> Party {
    Party {
        env: identity(ALPHA_KEY, "alpha"),
        deployment: support::deployment(ALPHA_BASE, deployment_key),
        request: support::simple_request("alpha-model"),
        model: "alpha-model",
    }
}

fn beta() -> Party {
    Party {
        env: identity(BETA_KEY, "beta"),
        deployment: support::deployment(BETA_BASE, Some(BETA_KEY)),
        request: support::simple_request("beta-model"),
        model: "beta-model",
    }
}

fn sent_header<'plan>(plan: &'plan Plan, name: &str) -> Option<&'plan str> {
    text_header(&chat_http(&plan.prepared).headers, name)
}

fn sent_model(plan: &Plan) -> JsonValue {
    let body: JsonValue =
        serde_json::from_slice(&chat_http(&plan.prepared).body).expect("the body");
    field(&body, "model").clone()
}

// Two identities keep their own key, base and headers, and a repeated call settles the same way
#[test]
fn foundry_provider_credential_identity_isolation() {
    let alpha_party = alpha(None);
    let beta_party = beta();
    let alpha_plan = alpha_party.plan(None);
    check_alpha_credentials(&alpha_plan);
    check_supplied_headers_replace_global(&alpha_party);
    let beta_plan = beta_party.plan(None);
    assert_eq!(beta_plan.base.as_deref(), Some(BETA_BASE));
    assert_eq!(sent_header(&beta_plan, "api-key"), Some(BETA_KEY));
    assert_eq!(sent_header(&beta_plan, "x-tenant"), Some("beta"));
    assert_eq!(
        sent_model(&alpha_plan),
        JsonValue::Text(String::from("alpha-model"))
    );
    assert_eq!(
        sent_model(&beta_plan),
        JsonValue::Text(String::from("beta-model"))
    );
    assert_ne!(
        chat_http(&alpha_plan.prepared).url,
        chat_http(&beta_plan.prepared).url
    );
    check_alpha_dispatch_after_beta(&alpha_party, &alpha_plan);
}

fn check_alpha_credentials(plan: &Plan) {
    assert_eq!(plan.base.as_deref(), Some(ALPHA_BASE));
    assert_eq!(
        plan.key.as_ref().map(|key| key.expose_secret().to_owned()),
        Some(String::from(ALPHA_KEY))
    );
    assert_eq!(sent_header(plan, "api-key"), Some(ALPHA_KEY));
    assert_eq!(sent_header(plan, "x-tenant"), Some("alpha"));
    assert!(sent_header(plan, "authorization").is_none());
}

// a supplied map replaces the global one, so the environment's tenant header never reaches this call
fn check_supplied_headers_replace_global(party: &Party) {
    let supplied = Headers::from([(String::from("x-own"), String::from("2"))]);
    let plan = party.plan(Some(&supplied));
    assert_eq!(sent_header(&plan, "x-own"), Some("2"));
    assert!(sent_header(&plan, "x-tenant").is_none());
}

// running the alpha identity after beta still sends only alpha's credentials
fn check_alpha_dispatch_after_beta(party: &Party, plan: &Plan) {
    let (result, logger) = party.run(support::supplied(200, REPLY));
    let outcome = result.expect("the alpha dispatch runs");
    let OperationResult::Chat(ChatCompletion::OpenAi(reply)) = outcome.result else {
        panic!("the alpha dispatch replies with a chat completion");
    };
    assert_eq!(reply.model, "azure_ai/provider-model");
    assert!(logger.calls.is_empty());
    assert!(
        !plan
            .key
            .as_ref()
            .expect("the alpha key")
            .expose_secret()
            .contains(BETA_KEY)
    );
}

// Debug and the owned failure paths carry the resolved facts but never the credential itself
#[test]
fn foundry_provider_secret_safe_failure() {
    let party = alpha(Some(ALPHA_KEY));
    let rendered = format!("{:?}", party.deployment);
    assert!(!rendered.contains(ALPHA_KEY));
    assert!(rendered.contains("REDACTED"));
    let plan = party.plan(None);
    let header = chat_http(&plan.prepared)
        .headers
        .get("api-key")
        .expect("the resolved key header");
    assert!(header.is_sensitive());
    check_failure_hides_key(&party);
}

fn check_failure_hides_key(party: &Party) {
    let (result, logger) = party.run(support::supplied(500, r#"{"error":"upstream is down"}"#));
    let Err(error) = result else {
        panic!("a 500 is a failure");
    };
    assert!(!error.to_string().contains(ALPHA_KEY));
    assert!(!format!("{error:?}").contains(ALPHA_KEY));
    assert_eq!(logger.calls.len(), 1);
    let Json::Text(original) = &logger.calls[0].original_response else {
        panic!("the failure log carries the error text");
    };
    assert!(!original.contains(ALPHA_KEY));
    let headers = logger.calls[0].headers.as_ref().expect("the headers");
    assert_eq!(
        headers.get("x-tenant"),
        Some(&Json::Text(String::from("alpha")))
    );
    assert!(
        !serde_json::to_string(headers)
            .expect("the headers text")
            .contains(ALPHA_KEY)
    );
}
