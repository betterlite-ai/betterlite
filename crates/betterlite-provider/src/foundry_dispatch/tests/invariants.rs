use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{self, Env, field};
use crate::foundry_dispatch::chat;
use crate::foundry_dispatch::fixtures::logger::RecordingLogger;
use crate::operation::{ChatCompletion, OperationResult};
use crate::prepared::PreparedRequest;

const REPLY: &str = r#"{"id":"chatcmpl-foundry","object":"chat.completion","created":1700000000,"model":"provider-model","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const ALPHA_KEY: &str = "key-alpha-0000000000";
const BETA_KEY: &str = "key-beta-1111111111";
const ALPHA_BASE: &str = "https://alpha.services.ai.azure.com/models";
const BETA_BASE: &str = "https://beta.openai.azure.com/models";

fn identity(key: &str, tenant: &str) -> Env {
    Env {
        global_api_key: Some(SecretString::from(key.to_owned())),
        headers: Some(Headers::from([(
            String::from("x-tenant"),
            tenant.to_owned(),
        )])),
        ..Env::default()
    }
}

fn prepared_headers(prepared: &PreparedRequest) -> &http::HeaderMap {
    let PreparedRequest::Chat(prepared) = prepared else {
        panic!("the chat call prepares a chat request");
    };
    &prepared.http.headers
}

fn text_header<'headers>(headers: &'headers http::HeaderMap, name: &str) -> Option<&'headers str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

// Two configured identities keep their own key, their own base and their own header map, and a
// call made twice settles the same way, so nothing resolved for one identity is reused by another
#[test]
fn foundry_provider_credential_identity_isolation() {
    support::block_on(async {
        let alpha_env = identity(ALPHA_KEY, "alpha");
        let alpha = support::deployment(ALPHA_BASE, None);
        let beta_env = identity(BETA_KEY, "beta");
        let beta = support::deployment(BETA_BASE, Some(BETA_KEY));

        let alpha_request = support::simple_request("alpha-model");
        let alpha_call = chat::Call {
            model: "alpha-model",
            request: &alpha_request,
            extra_headers: None,
            headers: None,
            api_base: Some(alpha.api_base.as_str()),
            api_key: alpha.api_key.as_ref(),
            deployment: &alpha,
            stream: false,
        };
        let alpha_plan = chat::prepare::prepare(&alpha_call, &alpha_env.globals())
            .expect("the alpha identity prepares");
        assert_eq!(alpha_plan.base.as_deref(), Some(ALPHA_BASE));
        assert_eq!(
            alpha_plan
                .key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            Some(String::from(ALPHA_KEY))
        );
        assert_eq!(
            text_header(prepared_headers(&alpha_plan.prepared), "api-key"),
            Some(ALPHA_KEY)
        );
        assert_eq!(
            text_header(prepared_headers(&alpha_plan.prepared), "x-tenant"),
            Some("alpha")
        );
        assert!(text_header(prepared_headers(&alpha_plan.prepared), "authorization").is_none());

        let beta_request = support::simple_request("beta-model");
        let beta_call = chat::Call {
            model: "beta-model",
            request: &beta_request,
            extra_headers: None,
            headers: None,
            api_base: Some(beta.api_base.as_str()),
            api_key: beta.api_key.as_ref(),
            deployment: &beta,
            stream: false,
        };
        let beta_plan = chat::prepare::prepare(&beta_call, &beta_env.globals())
            .expect("the beta identity prepares");
        assert_eq!(beta_plan.base.as_deref(), Some(BETA_BASE));
        assert_eq!(
            text_header(prepared_headers(&beta_plan.prepared), "api-key"),
            Some(BETA_KEY)
        );
        assert_eq!(
            text_header(prepared_headers(&beta_plan.prepared), "x-tenant"),
            Some("beta")
        );
        let PreparedRequest::Chat(alpha_prepared) = &alpha_plan.prepared else {
            panic!("the alpha call prepares a chat request");
        };
        let PreparedRequest::Chat(beta_prepared) = &beta_plan.prepared else {
            panic!("the beta call prepares a chat request");
        };
        let alpha_body: JsonValue =
            serde_json::from_slice(&alpha_prepared.http.body).expect("the alpha body");
        assert_eq!(
            field(&alpha_body, "model"),
            &JsonValue::Text(String::from("alpha-model"))
        );
        let beta_body: JsonValue =
            serde_json::from_slice(&beta_prepared.http.body).expect("the beta body");
        assert_eq!(
            field(&beta_body, "model"),
            &JsonValue::Text(String::from("beta-model"))
        );
        assert_ne!(alpha_prepared.http.url, beta_prepared.http.url);

        // running the alpha identity after beta still sends only alpha's credentials
        let mut logger = RecordingLogger::default();
        let outcome = chat::Dispatch {
            call: alpha_call,
            globals: alpha_env.globals(),
            answer: support::supplied(200, REPLY),
        }
        .run(&mut logger)
        .await
        .expect("the alpha dispatch runs");
        let OperationResult::Chat(ChatCompletion::OpenAi(reply)) = outcome.result else {
            panic!("the alpha dispatch replies with a chat completion");
        };
        assert_eq!(reply.model, "azure_ai/provider-model");
        assert!(logger.calls.is_empty());
        assert!(
            !alpha_plan
                .key
                .as_ref()
                .expect("the alpha key")
                .expose_secret()
                .contains(BETA_KEY)
        );
    });
}

// Debug and the owned failure paths carry the resolved facts but never the credential itself
#[test]
fn foundry_provider_secret_safe_failure() {
    support::block_on(async {
        let env = identity(ALPHA_KEY, "alpha");
        let deployment = support::deployment(ALPHA_BASE, Some(ALPHA_KEY));
        let rendered = format!("{deployment:?}");
        assert!(!rendered.contains(ALPHA_KEY));
        assert!(rendered.contains("REDACTED"));

        let request = support::simple_request("alpha-model");
        let call = chat::Call {
            model: "alpha-model",
            request: &request,
            extra_headers: None,
            headers: None,
            api_base: Some(deployment.api_base.as_str()),
            api_key: deployment.api_key.as_ref(),
            deployment: &deployment,
            stream: false,
        };
        let plan = chat::prepare::prepare(&call, &env.globals()).expect("the plan");
        let header = prepared_headers(&plan.prepared)
            .get("api-key")
            .expect("the resolved key header");
        assert!(header.is_sensitive());

        let mut logger = RecordingLogger::default();
        let error = chat::Dispatch {
            call,
            globals: env.globals(),
            answer: support::supplied(500, r#"{"error":"upstream is down"}"#),
        }
        .run(&mut logger)
        .await
        .expect_err("a 500 is a failure");
        assert!(!error.to_string().contains(ALPHA_KEY));
        assert!(!format!("{error:?}").contains(ALPHA_KEY));
        assert_eq!(logger.calls.len(), 1);
        let Json::Text(logged) = &logger.calls[0].original_response else {
            panic!("the failure log carries the error text");
        };
        assert!(!logged.contains(ALPHA_KEY));
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
    });
}
