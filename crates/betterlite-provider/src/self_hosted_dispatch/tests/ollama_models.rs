use std::fs;
use std::io::Error as IoError;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_models::ollama_model_metadata::{LiteLlmGlobals, OllamaSettings};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::self_hosted_dispatch::ollama_models::{
    OllamaModels, TAGS_TIMEOUT, TagsRequest, TagsResponse, TagsTransport, decode_tags,
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
    type Error = IoError;

    fn get_tags(&self, request: &TagsRequest) -> Result<TagsResponse, Self::Error> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        if self.failing {
            return Err(IoError::other("offline"));
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
        api_key: api_key.map(secret),
        api_base: api_base.map(str::to_owned),
    }
}

#[derive(Clone, Copy, Default)]
struct Passed<'passed> {
    base: Option<&'passed str>,
    key: Option<&'passed SecretString>,
}

fn list(
    settings: &OllamaSettings,
    globals: &LiteLlmGlobals,
    catalog: &[String],
    transport: &FakeTransport,
    passed: Passed<'_>,
) -> Vec<String> {
    OllamaModels {
        settings,
        globals,
        catalog,
        transport,
    }
    .get_models(passed.base, passed.key)
}

#[test]
fn listing_prefixes_sorts_and_dedupes_the_names() {
    let transport = FakeTransport::answering(
        200,
        r#"{"models":[{"name":"b"},{"name":"ollama/a"},{"model":"c"},{"name":""},{"name":"","model":"d"},{"name":7},{"model":""},3]}"#,
    );
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &[],
        &transport,
        Passed::default(),
    );
    assert_eq!(
        names,
        ["ollama/", "ollama/a", "ollama/b", "ollama/c", "ollama/d"]
    );
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
        Passed::default(),
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
        Passed::default(),
    );
    assert_eq!(names, ["ollama/m1", "ollama/m2"]);
}

#[rstest]
#[case::moved(300)]
#[case::not_found(404)]
#[case::server_error(500)]
fn listing_falls_back_on_a_non_2xx_status(#[case] status: u16) {
    let transport = FakeTransport::answering(status, r#"{"models":[{"name":"a"}]}"#);
    let catalog = vec![String::from("m1")];
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &catalog,
        &transport,
        Passed::default(),
    );
    assert_eq!(names, ["ollama/m1"], "{status}");
}

#[test]
fn listing_reads_a_2xx_answer() {
    let transport = FakeTransport::answering(204, r#"{"models":[{"name":"a"}]}"#);
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &Vec::new(),
        &transport,
        Passed::default(),
    );
    assert_eq!(names, ["ollama/a"], "a 2xx answer is read");
}

#[rstest]
#[case::empty_object("{}")]
#[case::models_not_a_list(r#"{"models":"x"}"#)]
#[case::bare_list("[]")]
#[case::bare_text("\"text\"")]
fn listing_returns_the_empty_list_for_bodies_without_models(#[case] body: &str) {
    let transport = FakeTransport::answering(200, body);
    let catalog = vec![String::from("m1")];
    let names = list(
        &ollama_settings(None, None),
        &no_globals(),
        &catalog,
        &transport,
        Passed::default(),
    );
    assert!(names.is_empty(), "{body}");
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
        Passed::default(),
    );
    assert_eq!(names, ["ollama/m1"]);
}

fn tags_url(passed: Option<&str>, section: &OllamaSettings, globals: &LiteLlmGlobals) -> String {
    let transport = FakeTransport::answering(200, "{}");
    let request = Passed {
        base: passed,
        key: None,
    };
    list(section, globals, &[], &transport, request);
    transport.requests().remove(0).url
}

#[rstest]
#[case::generate("/api/generate")]
#[case::chat("/api/chat")]
#[case::embed("/api/embed")]
#[case::embeddings("/api/embeddings")]
#[case::show("/api/show")]
#[case::tags("/api/tags")]
fn tags_request_strips_a_known_server_suffix(#[case] suffix: &str) {
    assert_eq!(
        tags_url(
            Some(format!("http://host:11434{suffix}").as_str()),
            &ollama_settings(None, None),
            &no_globals()
        ),
        "http://host:11434/api/tags",
        "{suffix}"
    );
}

#[test]
fn tags_request_targets_the_server_base() {
    assert_eq!(
        tags_url(
            Some("http://host:11434"),
            &ollama_settings(None, None),
            &no_globals()
        ),
        "http://host:11434/api/tags"
    );
    assert_eq!(
        tags_url(
            Some("http://host:11434/"),
            &ollama_settings(None, None),
            &no_globals()
        ),
        "http://host:11434/api/tags"
    );
    assert_eq!(
        tags_url(
            None,
            &ollama_settings(Some("http://env"), None),
            &no_globals()
        ),
        "http://env/api/tags"
    );
    assert_eq!(
        tags_url(
            Some(""),
            &ollama_settings(Some("http://env"), None),
            &no_globals()
        ),
        "http://env/api/tags",
        "an empty passed base falls through"
    );
    assert_eq!(
        tags_url(
            Some("http://passed"),
            &ollama_settings(Some("http://env"), None),
            &no_globals()
        ),
        "http://passed/api/tags"
    );
}

#[test]
fn tags_request_targets_the_local_server_when_no_base_is_set() {
    assert_eq!(
        tags_url(None, &ollama_settings(None, None), &no_globals()),
        "http://localhost:11434/api/tags"
    );
}

fn tags_headers(
    passed_base: Option<&str>,
    passed_key: Option<&str>,
    section: &OllamaSettings,
    globals: &LiteLlmGlobals,
) -> Option<String> {
    let transport = FakeTransport::answering(200, "{}");
    let key = passed_key.map(secret);
    let request = Passed {
        base: passed_base,
        key: key.as_ref(),
    };
    list(section, globals, &[], &transport, request);
    transport
        .requests()
        .remove(0)
        .headers
        .into_iter()
        .find(|(name, _)| *name == "Authorization")
        .map(|(_, value)| value.expose_secret().to_owned())
}

#[test]
fn tags_request_carries_the_first_resolved_key() {
    let section = ollama_settings(None, Some("sk-env"));
    assert_eq!(
        tags_headers(None, Some("sk-passed"), &section, &no_globals()),
        Some(String::from("Bearer sk-passed"))
    );
    assert_eq!(
        tags_headers(None, Some(""), &section, &no_globals()),
        Some(String::from("Bearer sk-env")),
        "an empty passed key falls through"
    );
}

#[test]
fn tags_request_falls_through_the_litellm_globals_for_a_key() {
    let section = ollama_settings(None, None);
    let every_global = LiteLlmGlobals {
        api_key: Some(secret("sk-litellm")),
        openai_key: Some(secret("sk-openai")),
        secret_manager_api_key: Some(secret("sk-secret")),
    };
    assert_eq!(
        tags_headers(None, None, &section, &every_global),
        Some(String::from("Bearer sk-litellm"))
    );
    let openai_and_secret = LiteLlmGlobals {
        openai_key: Some(secret("sk-openai")),
        secret_manager_api_key: Some(secret("sk-secret")),
        ..no_globals()
    };
    assert_eq!(
        tags_headers(None, None, &section, &openai_and_secret),
        Some(String::from("Bearer sk-openai"))
    );
    let secret_only = LiteLlmGlobals {
        secret_manager_api_key: Some(secret("sk-secret")),
        ..no_globals()
    };
    assert_eq!(
        tags_headers(None, None, &section, &secret_only),
        Some(String::from("Bearer sk-secret"))
    );
}

#[test]
fn tags_request_drops_every_key_for_an_explicit_base() {
    let section = ollama_settings(None, Some("sk-env"));
    assert_eq!(
        tags_headers(None, None, &ollama_settings(None, None), &no_globals()),
        None,
        "no key means no header"
    );
    assert_eq!(
        tags_headers(Some("http://passed"), None, &section, &no_globals()),
        None,
        "a passed base without its own key drops every key"
    );
    assert_eq!(
        tags_headers(Some("http://passed"), Some(""), &section, &no_globals()),
        None,
        "an empty passed key is no key, so an explicit base drops the environment key too"
    );
    assert_eq!(
        tags_headers(
            Some("http://passed"),
            Some("sk-passed"),
            &section,
            &no_globals()
        ),
        Some(String::from("Bearer sk-passed"))
    );
}

#[test]
fn tags_request_keeps_the_httpx_default_timeout() {
    assert_eq!(TAGS_TIMEOUT, Duration::from_secs(5));
}

// every corpus file goes through the tags decoder the fuzzer feeds
#[test]
fn replays_the_ollama_tags_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/ollama_tags_reply")
        .expect("the tags corpus exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).expect("the corpus file reads");
            let _tags = decode_tags(&bytes);
        })
        .count();
    assert!(seen > 0_usize);
}
