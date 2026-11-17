use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::compatible_embedding::{EmbeddingCall as SharedEmbeddingCall, Mode};
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::self_hosted_dispatch::call::{EmbeddingProvider, EmbeddingRequest};
use crate::self_hosted_dispatch::embedding::dispatch;
use crate::self_hosted_dispatch::error::DispatchError;

use super::support::{Fixture, SectionFixture, environment, secret, secret_text};

fn request(provider: ProviderId, fixture: &Fixture) -> Result<EmbeddingRequest, DispatchError> {
    let call = fixture.embedding_call();
    let context = fixture.embedding_context();
    dispatch(provider, &call, &context).expect("the dispatch owns this provider")
}

fn compatible(
    provider: ProviderId,
    fixture: &Fixture,
) -> (
    EmbeddingProvider,
    Option<String>,
    Option<String>,
    EmbeddingInput,
    Object,
    Option<Vec<(String, String)>>,
) {
    let resolution = request(provider, fixture).expect("the arm answers");
    let EmbeddingRequest::Compatible {
        provider: resolved_provider,
        api_base,
        api_key,
        input,
        optional_params,
        headers,
    } = resolution
    else {
        panic!("{provider:?} resolves a compatible request");
    };
    let headers = headers.map(|given| given.into_iter().collect());
    (
        resolved_provider,
        api_base,
        secret_text(api_key.as_ref()),
        input,
        optional_params,
        headers,
    )
}

#[test]
fn embedding_dispatch_leaves_other_providers_alone() {
    let fixture = Fixture::new();
    let call = fixture.embedding_call();
    let context = fixture.embedding_context();
    for provider in [ProviderId::Openai, ProviderId::OpenaiLike] {
        assert!(
            dispatch(provider, &call, &context).is_none(),
            "{provider:?}"
        );
    }
}

fn hosted_vllm_base(passed: Option<&str>, section: SectionFixture) -> Option<String> {
    let mut fixture = Fixture::new();
    fixture.api_base = passed.map(str::to_owned);
    fixture.configure(section);
    compatible(ProviderId::HostedVllm, &fixture).1
}

#[test]
fn hosted_vllm_embedding_base_falls_from_the_passed_value_to_the_environment_then_litellm() {
    assert_eq!(
        hosted_vllm_base(
            Some("http://passed"),
            SectionFixture {
                hosted_vllm_api_base: Some(String::from("http://env")),
                api_base: Some(String::from("http://litellm")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("http://passed"))
    );
    assert_eq!(
        hosted_vllm_base(
            None,
            SectionFixture {
                hosted_vllm_api_base: Some(String::from("http://env")),
                api_base: Some(String::from("http://litellm")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("http://env"))
    );
    assert_eq!(
        hosted_vllm_base(
            Some(""),
            SectionFixture {
                hosted_vllm_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("http://env")),
        "an empty passed base falls through"
    );
    assert_eq!(
        hosted_vllm_base(
            None,
            SectionFixture {
                api_base: Some(String::from("http://litellm")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("http://litellm"))
    );
    assert_eq!(hosted_vllm_base(None, SectionFixture::default()), None);
}

fn hosted_vllm_key(passed: Option<&str>, section: SectionFixture) -> Option<String> {
    let mut fixture = Fixture::new();
    fixture.api_key = passed.and_then(secret);
    fixture.configure(section);
    compatible(ProviderId::HostedVllm, &fixture).2
}

#[test]
fn hosted_vllm_embedding_key_falls_from_the_passed_value_to_the_environment() {
    assert_eq!(
        hosted_vllm_key(
            Some("sk-passed"),
            SectionFixture {
                hosted_vllm_api_key: Some(String::from("sk-env")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("sk-passed"))
    );
    assert_eq!(
        hosted_vllm_key(
            None,
            SectionFixture {
                hosted_vllm_api_key: Some(String::from("sk-env")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("sk-env"))
    );
    assert_eq!(
        hosted_vllm_key(
            Some(""),
            SectionFixture {
                hosted_vllm_api_key: Some(String::from("sk-env")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("sk-env")),
        "an empty passed key falls through"
    );
    assert_eq!(
        hosted_vllm_key(
            None,
            SectionFixture {
                api_key: Some(String::from("sk-litellm")),
                ..SectionFixture::default()
            }
        ),
        None
    );
}

#[test]
fn hosted_vllm_embedding_carries_the_input_headers_and_params() {
    let mut fixture = Fixture::new();
    fixture.input = EmbeddingInput::Texts(vec![String::from("a"), String::from("b")]);
    fixture.headers = Some([(String::from("X-Extra"), String::from("one"))].into());
    fixture
        .optional_params
        .insert(String::from("dimensions"), Json::Number(8.into()));
    let (provider, _base, _key, input, optional_params, headers) =
        compatible(ProviderId::HostedVllm, &fixture);
    assert_eq!(provider, EmbeddingProvider::HostedVllm);
    assert_eq!(
        input,
        EmbeddingInput::Texts(vec![String::from("a"), String::from("b")])
    );
    assert_eq!(
        optional_params.get("dimensions"),
        Some(&Json::Number(8.into()))
    );
    assert_eq!(
        headers,
        Some(vec![(String::from("X-Extra"), String::from("one"))])
    );
}

#[test]
fn triton_embedding_requires_the_passed_base() {
    let fixture = Fixture::new();
    let error = request(ProviderId::Triton, &fixture).expect_err("triton needs a base");
    assert_eq!(
        error.to_string(),
        "api_base is required for triton. Please pass `api_base`"
    );
    assert_eq!(error.status(), Some(400));

    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("http://passed"));
    fixture.api_key = secret("sk-passed");
    fixture.headers = Some([(String::from("X-Extra"), String::from("one"))].into());
    let (provider, api_base, api_key, _input, _params, headers) =
        compatible(ProviderId::Triton, &fixture);
    assert_eq!(provider, EmbeddingProvider::Triton);
    assert_eq!(api_base, Some(String::from("http://passed")));
    assert_eq!(api_key, Some(String::from("sk-passed")));
    assert!(
        headers.is_none(),
        "the triton arm never sends the caller's headers"
    );
}

#[test]
fn oobabooga_embedding_keeps_the_values_it_was_given() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("http://passed"));
    fixture.api_key = secret("sk-passed");
    fixture.headers = Some([(String::from("X-Extra"), String::from("one"))].into());
    fixture.configure(SectionFixture {
        api_base: Some(String::from("http://litellm")),
        api_key: Some(String::from("sk-litellm")),
        ..SectionFixture::default()
    });
    let (provider, api_base, api_key, _input, _params, headers) =
        compatible(ProviderId::Oobabooga, &fixture);
    assert_eq!(provider, EmbeddingProvider::Oobabooga);
    assert_eq!(api_base, Some(String::from("http://passed")));
    assert_eq!(api_key, Some(String::from("sk-passed")));
    assert!(headers.is_none());
}

fn ollama_prompts(input: EmbeddingInput) -> Result<Vec<String>, DispatchError> {
    let mut fixture = Fixture::new();
    fixture.input = input;
    let resolution = request(ProviderId::Ollama, &fixture)?;
    let EmbeddingRequest::Ollama { prompts, .. } = resolution else {
        panic!("ollama resolves its own request");
    };
    Ok(prompts)
}

#[test]
fn ollama_embedding_lists_prompts_from_text_or_texts() {
    assert_eq!(
        ollama_prompts(EmbeddingInput::Text(String::from("hello"))).expect("one prompt"),
        vec![String::from("hello")]
    );
    assert_eq!(
        ollama_prompts(EmbeddingInput::Texts(vec![
            String::from("a"),
            String::from("b"),
        ]))
        .expect("two prompts"),
        vec![String::from("a"), String::from("b")]
    );
    assert_eq!(
        ollama_prompts(EmbeddingInput::Texts(Vec::new())).expect("an empty prompt list"),
        Vec::<String>::new()
    );
    assert_eq!(
        ollama_prompts(EmbeddingInput::Tokens(Vec::new()))
            .expect("an empty token list counts as no prompts"),
        Vec::<String>::new()
    );
    assert_eq!(
        ollama_prompts(EmbeddingInput::TokenLists(Vec::new()))
            .expect("an empty token-list list counts as no prompts"),
        Vec::<String>::new()
    );
}

#[test]
fn ollama_embedding_rejects_token_input() {
    for input in [
        EmbeddingInput::Tokens(vec![1, 2]),
        EmbeddingInput::TokenLists(vec![vec![1, 2]]),
    ] {
        let error = ollama_prompts(input).expect_err("tokens are not prompts");
        assert_eq!(error.status(), Some(400));
        assert!(matches!(error, DispatchError::OllamaEmbeddingInput));
    }
}

fn ollama_embedding_base(passed: Option<&str>, section: SectionFixture) -> String {
    let mut fixture = Fixture::new();
    fixture.api_base = passed.map(str::to_owned);
    fixture.configure(section);
    fixture
        .optional_params
        .insert(String::from("truncate"), Json::Bool(true));
    let resolution = request(ProviderId::Ollama, &fixture).expect("the arm answers");
    let EmbeddingRequest::Ollama {
        api_base,
        model,
        optional_params,
        ..
    } = resolution
    else {
        panic!("ollama resolves its own request");
    };
    assert_eq!(model, "model");
    assert_eq!(optional_params.get("truncate"), Some(&Json::Bool(true)));
    api_base
}

#[test]
fn ollama_embedding_reads_the_chat_base_chain() {
    assert_eq!(
        ollama_embedding_base(
            Some("http://passed"),
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                api_base: Some(String::from("http://litellm")),
                ..SectionFixture::default()
            }
        ),
        "http://litellm"
    );
    assert_eq!(
        ollama_embedding_base(
            Some("http://passed"),
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        "http://passed"
    );
    assert_eq!(
        ollama_embedding_base(
            None,
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        "http://env"
    );
    assert_eq!(
        ollama_embedding_base(
            Some(""),
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        "http://env"
    );
    assert_eq!(
        ollama_embedding_base(None, SectionFixture::default()),
        "http://localhost:11434"
    );
}

fn xinference(
    passed: Option<(&str, &str)>,
    section: SectionFixture,
    pairs: &[(&str, &str)],
) -> (Option<String>, Option<String>) {
    let mut fixture = Fixture::new();
    if let Some((base, key)) = passed {
        fixture.api_base = Some(base.to_owned());
        fixture.api_key = secret(key);
    }
    fixture.configure(section);
    fixture.environment = environment(pairs);
    let (_provider, api_base, api_key, _input, _params, _headers) =
        compatible(ProviderId::Xinference, &fixture);
    (api_base, api_key)
}

#[test]
fn xinference_embedding_reads_the_environment_then_the_stubs() {
    let nothing = || SectionFixture::default();
    assert_eq!(
        xinference(None, nothing(), &[]),
        (
            Some(String::from("http://127.0.0.1:9997/v1")),
            Some(String::from("stub-xinference-key"))
        )
    );
    assert_eq!(
        xinference(
            None,
            nothing(),
            &[
                ("XINFERENCE_API_BASE", "http://env"),
                ("XINFERENCE_API_KEY", "sk-env"),
            ]
        ),
        (
            Some(String::from("http://env")),
            Some(String::from("sk-env"))
        )
    );
    assert_eq!(
        xinference(
            None,
            nothing(),
            &[("XINFERENCE_API_BASE", ""), ("XINFERENCE_API_KEY", "")]
        ),
        (
            Some(String::from("http://127.0.0.1:9997/v1")),
            Some(String::from("stub-xinference-key"))
        ),
        "an empty environment value counts as unset"
    );
    assert_eq!(
        xinference(
            Some(("http://passed", "sk-passed")),
            SectionFixture {
                api_base: Some(String::from("http://litellm")),
                api_key: Some(String::from("sk-litellm")),
                ..SectionFixture::default()
            },
            &[
                ("XINFERENCE_API_BASE", "http://env"),
                ("XINFERENCE_API_KEY", "sk-env"),
            ]
        ),
        (
            Some(String::from("http://passed")),
            Some(String::from("sk-passed"))
        )
    );
    assert_eq!(
        xinference(
            Some(("", "")),
            SectionFixture {
                api_base: Some(String::from("http://litellm")),
                api_key: Some(String::from("sk-litellm")),
                ..SectionFixture::default()
            },
            &[
                ("XINFERENCE_API_BASE", "http://env"),
                ("XINFERENCE_API_KEY", "sk-env"),
            ]
        ),
        (
            Some(String::from("http://litellm")),
            Some(String::from("sk-litellm"))
        ),
        "empty passed values fall through to litellm's globals"
    );
}

fn shared_call(
    provider: ProviderId,
    fixture: &Fixture,
) -> (EmbeddingProvider, SharedEmbeddingCall) {
    let resolution = request(provider, fixture).expect("the arm answers");
    let EmbeddingRequest::Shared {
        provider: resolved_provider,
        call,
    } = resolution
    else {
        panic!("{provider:?} resolves through the shared openai_like call");
    };
    (resolved_provider, *call)
}

#[test]
fn llamafile_embedding_prepares_the_shared_openai_like_call() {
    let mut fixture = Fixture::new();
    fixture.openai_like(Some("http://like"), Some("sk-like"));
    fixture.input = EmbeddingInput::Text(String::from("hello"));
    fixture.optional_params.insert(
        String::from("encoding_format"),
        Json::Text(String::from("float")),
    );
    let (provider, call) = shared_call(ProviderId::Llamafile, &fixture);
    assert_eq!(provider, EmbeddingProvider::Llamafile);
    assert_eq!(call.model, "model");
    assert_eq!(call.api_base, Some(String::from("http://like")));
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        Some(String::from("sk-like"))
    );
    assert_eq!(call.input, DecisionsJson::Text(String::from("hello")));
    assert_eq!(
        call.options.get("encoding_format"),
        Some(&DecisionsJson::Text(String::from("float")))
    );
    assert!(call.headers.is_none());
    assert!(!call.custom_endpoint);
    assert_eq!(call.timeout, Duration::from_secs(600));
    assert_eq!(call.mode, Mode::Sync);
}

#[test]
fn lm_studio_embedding_takes_the_async_mode_from_the_call() {
    let mut fixture = Fixture::new();
    fixture.aembedding = true;
    fixture.globals.api_base = Some(String::from("http://global"));
    fixture.globals.openai_like_key = Some(String::from("sk-global"));
    fixture.input = EmbeddingInput::Texts(vec![String::from("a")]);
    let (provider, call) = shared_call(ProviderId::LmStudio, &fixture);
    assert_eq!(provider, EmbeddingProvider::LmStudio);
    assert_eq!(call.mode, Mode::Async);
    assert_eq!(call.api_base, Some(String::from("http://global")));
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        Some(String::from("sk-global"))
    );
    assert_eq!(
        call.input,
        DecisionsJson::List(vec![DecisionsJson::Text(String::from("a"))])
    );
}

#[test]
fn shared_embedding_moves_the_headers_into_the_optional_params() {
    let mut fixture = Fixture::new();
    fixture.openai_like(Some("http://like"), Some("sk-like"));
    fixture.headers = Some([(String::from("X-Extra"), String::from("one"))].into());
    let (_provider, call) = shared_call(ProviderId::Llamafile, &fixture);
    let Some(DecisionsJson::Object(extra)) = call.options.get("extra_headers") else {
        panic!("the shared call puts the headers into extra_headers");
    };
    assert_eq!(
        extra.get("X-Extra"),
        Some(&DecisionsJson::Text(String::from("one")))
    );
    assert!(call.headers.is_none());

    let mut fixture = Fixture::new();
    fixture.headers = Some(BTreeMap::new());
    let (_provider, call) = shared_call(ProviderId::Llamafile, &fixture);
    assert!(
        call.options.get("extra_headers").is_none(),
        "empty headers add nothing"
    );
}

#[test]
fn embedding_requests_report_the_provider_in_debug() {
    let fixture = Fixture::new();
    let compatible = request(ProviderId::HostedVllm, &fixture).expect("the arm answers");
    assert!(format!("{compatible:?}").contains("HostedVllm"));
    let ollama = request(ProviderId::Ollama, &fixture).expect("the arm answers");
    assert!(format!("{ollama:?}").contains("Ollama"));
    let shared = request(ProviderId::Llamafile, &fixture).expect("the arm answers");
    assert!(format!("{shared:?}").contains("Llamafile"));
}
