use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::ollama_model_metadata::{LiteLlmGlobals, OllamaSettings};
use secrecy::{ExposeSecret as _, SecretString};

use crate::self_hosted_dispatch::ollama_models::{
    OllamaModels, TAGS_TIMEOUT, TagsRequest, TagsResponse, TagsTransport, base_model,
    validate_environment,
};

use super::support::secret;

struct FakeTransport {
    status: u16,
    body: Vec<u8>,
    failing: bool,
    requests: Mutex<Vec<TagsRequest>>,
}

impl FakeTransport {
    fn answering(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            failing: false,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            status: 200,
            body: Vec::new(),
            failing: true,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<TagsRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TagsTransport for FakeTransport {
    type Error = std::io::Error;

    fn get_tags(&self, request: &TagsRequest) -> Result<TagsResponse, Self::Error> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        if self.failing {
            return Err(std::io::Error::other("offline"));
        }
        Ok(TagsResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

fn no_globals() -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_key: None,
        openai_key: None,
        secret_manager_api_key: None,
    }
}

fn ollama_settings(api_base: Option<&str>, api_key: Option<&str>) -> OllamaSettings {
    OllamaSettings {
        api_key: api_key.and_then(secret),
        api_base: api_base.map(str::to_owned),
    }
}

fn list(
    settings: &OllamaSettings,
    globals: &LiteLlmGlobals,
    catalog: &[String],
    transport: &FakeTransport,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> Vec<String> {
    OllamaModels {
        settings,
        globals,
        catalog,
        transport,
    }
    .get_models(api_base, api_key)
}

#[test]
fn listing_prefixes_sorts_and_dedupes_the_names() {
    let transport = FakeTransport::answering(
        200,
        r#"{"models":[{"name":"b"},{"name":"ollama/a"},{"model":"c"},{"name":""},{"name":7},{"model":""},3]}"#,
    );
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &[],
        &transport,
        None,
        None,
    );
    assert_eq!(names, ["ollama/", "ollama/a", "ollama/b", "ollama/c"]);
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://localhost:11434/api/tags");
    assert!(requests[0].headers.is_empty());
    assert_eq!(requests[0].timeout, TAGS_TIMEOUT);
}

#[test]
fn listing_accepts_a_bare_list() {
    let transport = FakeTransport::answering(200, r#"[{"name":"a"}]"#);
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &[],
        &transport,
        None,
        None,
    );
    assert_eq!(names, ["ollama/a"]);
}

#[test]
fn listing_falls_back_to_the_catalog_after_a_transport_failure() {
    let transport = FakeTransport::failing();
    let catalog = vec![String::from("m1"), String::from("m2")];
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &catalog,
        &transport,
        None,
        None,
    );
    assert_eq!(names, ["ollama/m1", "ollama/m2"]);
}

#[test]
fn listing_falls_back_on_a_non_2xx_status() {
    let catalog = vec![String::from("m1")];
    for status in [300, 404, 500] {
        let transport = FakeTransport::answering(status, r#"{"models":[{"name":"a"}]}"#);
        let names = list(
            &ollama_settings(None, None),
            &no_globals(),
            &catalog,
            &transport,
            None,
            None,
        );
        assert_eq!(names, ["ollama/m1"], "{status}");
    }
    let transport = FakeTransport::answering(204, r#"{"models":[{"name":"a"}]}"#);
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &Vec::new(),
        &transport,
        None,
        None,
    );
    assert_eq!(names, ["ollama/a"], "a 2xx answer is read");
}

#[test]
fn listing_returns_the_empty_list_for_bodies_without_models() {
    for body in ["{}", r#"{"models":"x"}"#, "[]", "\"text\""] {
        let transport = FakeTransport::answering(200, body);
        let catalog = vec![String::from("m1")];
        let names = list(
            &ollama_settings(None, None),
            &no_globals(),
            &catalog,
            &transport,
            None,
            None,
        );
        assert!(names.is_empty(), "{body}");
    }
}

#[test]
fn listing_falls_back_when_the_body_is_not_json() {
    let transport = FakeTransport::answering(200, "not json");
    let catalog = vec![String::from("m1")];
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &catalog,
        &transport,
        None,
        None,
    );
    assert_eq!(names, ["ollama/m1"]);
}

fn tags_url(passed: Option<&str>, section: OllamaSettings, globals: LiteLlmGlobals) -> String {
    let transport = FakeTransport::answering(200, "{}");
    list(&section, &globals, &[], &transport, passed, None);
    transport.requests().remove(0).url
}

#[test]
fn tags_request_targets_the_server_base() {
    assert_eq!(
        tags_url(
            Some("http://host:11434"),
            ollama_settings(None, None),
            no_globals()
        ),
        "http://host:11434/api/tags"
    );
    for suffix in [
        "/api/generate",
        "/api/chat",
        "/api/embed",
        "/api/embeddings",
        "/api/show",
        "/api/tags",
    ] {
        assert_eq!(
            tags_url(
                Some(format!("http://host:11434{suffix}").as_str()),
                ollama_settings(None, None),
                no_globals()
            ),
            "http://host:11434/api/tags",
            "{suffix}"
        );
    }
    assert_eq!(
        tags_url(
            Some("http://host:11434/"),
            ollama_settings(None, None),
            no_globals()
        ),
        "http://host:11434/api/tags"
    );
    assert_eq!(
        tags_url(
            None,
            ollama_settings(Some("http://env"), None),
            no_globals()
        ),
        "http://env/api/tags"
    );
    assert_eq!(
        tags_url(
            Some(""),
            ollama_settings(Some("http://env"), None),
            no_globals()
        ),
        "http://env/api/tags",
        "an empty passed base falls through"
    );
    assert_eq!(
        tags_url(
            Some("http://passed"),
            ollama_settings(Some("http://env"), None),
            no_globals()
        ),
        "http://passed/api/tags"
    );
    assert_eq!(
        tags_url(None, ollama_settings(None, None), no_globals()),
        "http://localhost:11434/api/tags"
    );
}

fn tags_headers(
    passed_base: Option<&str>,
    passed_key: Option<&str>,
    section: OllamaSettings,
    globals: LiteLlmGlobals,
) -> Vec<(String, String)> {
    let transport = FakeTransport::answering(200, "{}");
    let key = passed_key.and_then(secret);
    list(
        &section,
        &globals,
        &[],
        &transport,
        passed_base,
        key.as_ref(),
    );
    transport
        .requests()
        .remove(0)
        .headers
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.expose_secret().to_owned()))
        .collect()
}

#[test]
fn tags_request_carries_the_first_resolved_key() {
    let expected = |key: &str| vec![(String::from("Authorization"), format!("Bearer {key}"))];
    assert_eq!(
        tags_headers(
            None,
            Some("sk-passed"),
            ollama_settings(None, Some("sk-env")),
            no_globals()
        ),
        expected("sk-passed")
    );
    assert_eq!(
        tags_headers(
            None,
            Some(""),
            ollama_settings(None, Some("sk-env")),
            no_globals()
        ),
        expected("sk-env"),
        "an empty passed key falls through"
    );
    assert_eq!(
        tags_headers(
            None,
            None,
            ollama_settings(None, None),
            LiteLlmGlobals {
                api_key: secret("sk-litellm"),
                openai_key: secret("sk-openai"),
                secret_manager_api_key: secret("sk-secret"),
            }
        ),
        expected("sk-litellm")
    );
    assert_eq!(
        tags_headers(
            None,
            None,
            ollama_settings(None, None),
            LiteLlmGlobals {
                api_key: None,
                openai_key: secret("sk-openai"),
                secret_manager_api_key: secret("sk-secret"),
            }
        ),
        expected("sk-openai")
    );
    assert_eq!(
        tags_headers(
            None,
            None,
            ollama_settings(None, None),
            LiteLlmGlobals {
                api_key: None,
                openai_key: None,
                secret_manager_api_key: secret("sk-secret"),
            }
        ),
        expected("sk-secret")
    );
    assert!(
        tags_headers(None, None, ollama_settings(None, None), no_globals()).is_empty(),
        "no key means no header"
    );
    assert!(
        tags_headers(
            Some("http://passed"),
            None,
            ollama_settings(None, Some("sk-env")),
            no_globals()
        )
        .is_empty(),
        "a passed base without its own key drops every key"
    );
    assert_eq!(
        tags_headers(
            Some("http://passed"),
            Some("sk-passed"),
            ollama_settings(None, Some("sk-env")),
            no_globals()
        ),
        expected("sk-passed")
    );
}

#[test]
fn tags_request_keeps_the_httpx_default_timeout() {
    assert_eq!(TAGS_TIMEOUT, Duration::from_secs(5));
}

#[test]
fn validate_environment_adds_nothing() {
    assert!(validate_environment().is_empty());
}

#[test]
fn base_model_returns_the_model() {
    assert_eq!(base_model("ollama/llama3"), "ollama/llama3");
}
