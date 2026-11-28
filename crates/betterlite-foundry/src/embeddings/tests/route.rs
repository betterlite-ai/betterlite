use std::time::Duration;

use crate::embeddings::error::FoundryEmbeddingError;
use crate::embeddings::route::models_route_base;
use betterlite_http::client_kind::ClientKind;

use super::support::{
    Answer, BASE, Factory, IMAGE_ONE, IMAGE_URI, MODEL, TEXT_EMPTY, TEXT_ONE, Text, key, request,
    run, strings,
};

pub(super) fn foundry_models_embeddings_route() {
    route_bases();
    route_ignores_other_bases();
    route_keeps_query_and_fragment();
    image_url();
    sync_guards();
    sync_text_only_has_no_guard();
    text_call_gets_the_route_base();
    async_without_guards();
}

fn route_bases() {
    assert_eq!(models_route_base(None), None);
    assert_eq!(
        models_route_base(Some(BASE)),
        Some(format!("{BASE}/models"))
    );
    assert_eq!(
        models_route_base(Some(&format!("{BASE}/"))),
        Some(format!("{BASE}/models"))
    );
    assert_eq!(
        models_route_base(Some(&format!("{BASE}/v1"))),
        Some(format!("{BASE}/v1/models"))
    );
    assert_eq!(
        models_route_base(Some(&format!("{BASE}/models"))),
        Some(format!("{BASE}/models"))
    );
    assert_eq!(
        models_route_base(Some(&format!("{BASE}/models/"))),
        Some(format!("{BASE}/models/"))
    );
}

fn route_ignores_other_bases() {
    let openai = "https://api.openai.com/v1";
    assert_eq!(models_route_base(Some(openai)), Some(openai.to_owned()));
    let azure = "https://acct.openai.azure.com/v1";
    assert_eq!(models_route_base(Some(azure)), Some(azure.to_owned()));
    let deployments = format!("{BASE}/openai/deployments/deployment/v1");
    assert_eq!(
        models_route_base(Some(&deployments)),
        Some(deployments.clone())
    );
}

fn route_keeps_query_and_fragment() {
    let query = format!("{BASE}/v1?api-version=2024-05-01-preview");
    assert_eq!(
        models_route_base(Some(&query)),
        Some(format!("{BASE}/v1/models?api-version=2024-05-01-preview"))
    );
    let fragment = format!("{BASE}/v1#section");
    assert_eq!(
        models_route_base(Some(&fragment)),
        Some(format!("{BASE}/v1/models#section"))
    );
    let routed = format!("{BASE}/models?api-version=2024-05-01-preview#section");
    assert_eq!(models_route_base(Some(&routed)), Some(routed.clone()));
    let bare = format!("{BASE}?api-version=2024-05-01-preview");
    assert_eq!(
        models_route_base(Some(&bare)),
        Some(format!("{BASE}/models?api-version=2024-05-01-preview"))
    );
    let upper = "HTTPS://acct.services.ai.azure.com/v1";
    assert_eq!(
        models_route_base(Some(upper)),
        Some("https://acct.services.ai.azure.com/v1/models".to_owned())
    );
}

fn image_url() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let text = Text::replying(TEXT_EMPTY);
    let response = run(&call, None, &factory, &text).unwrap();
    assert_eq!(response.data.len(), 1);
    let requests = factory.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, format!("{BASE}/models/images/embeddings"));
    let queried = format!("{BASE}?api-version=2024-05-01-preview");
    call.api_base = Some(&queried);
    let queried_factory = Factory::new(Answer::json(IMAGE_ONE));
    run(&call, None, &queried_factory, &text).unwrap();
    let routed = format!("{BASE}/models?api-version=2024-05-01-preview");
    assert_eq!(
        queried_factory.requests()[0].url,
        format!("{routed}/images/embeddings")
    );
}

fn sync_guards() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("case-key");
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let text = Text::replying(TEXT_EMPTY);
    let mut call = request(MODEL, &input);
    call.api_key = Some(&secret);
    call.api_base = None;
    let missing_base = run(&call, None, &factory, &text).unwrap_err();
    assert!(matches!(
        missing_base,
        FoundryEmbeddingError::ApiBaseMissing
    ));
    assert_eq!(
        missing_base.to_string(),
        "api_base is None. Please set AZURE_AI_API_BASE or dynamically via `api_base` param, to make the request."
    );
    assert!(factory.requests().is_empty());
    assert!(factory.sync_timeouts().is_empty());
    call.api_base = Some(BASE);
    call.api_key = None;
    let missing_key = run(&call, None, &factory, &text).unwrap_err();
    assert!(matches!(missing_key, FoundryEmbeddingError::ApiKeyMissing));
    assert_eq!(
        missing_key.to_string(),
        "api_key is None. Please set AZURE_AI_API_KEY or dynamically via `api_key` param, to make the request."
    );
    assert!(factory.requests().is_empty());
}

fn sync_text_only_has_no_guard() {
    let input = strings(&["hello"]);
    let mut call = request(MODEL, &input);
    call.api_base = None;
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let text = Text::replying(TEXT_ONE);
    let response = run(&call, None, &factory, &text).unwrap();
    assert_eq!(response.data.len(), 1);
    assert_eq!(text.calls()[0].api_base, None);
    assert!(factory.requests().is_empty());
}

// the inherited text call gets the /models base and the key, in the caller's mode
fn text_call_gets_the_route_base() {
    let input = strings(&["hello"]);
    let secret = key("text-key");
    let base = format!("{BASE}/v1");
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        let mut call = request(MODEL, &input);
        call.mode = mode;
        call.api_base = Some(&base);
        call.api_key = Some(&secret);
        let factory = Factory::new(Answer::json(IMAGE_ONE));
        let text = Text::replying(TEXT_ONE);
        run(&call, None, &factory, &text).unwrap();
        let calls = text.calls();
        let seen = &calls[0];
        assert_eq!(seen.api_base, Some(format!("{BASE}/v1/models")));
        assert_eq!(seen.api_key.as_deref(), Some("text-key"));
        assert_eq!(seen.mode, mode);
        assert_eq!(seen.model.as_str(), MODEL);
        assert_eq!(seen.timeout, Duration::from_secs(5));
    }
}

fn async_without_guards() {
    let input = strings(&[IMAGE_URI]);
    let secret = key("async-key");
    let mut call = request(MODEL, &input);
    call.mode = ClientKind::Asynchronous;
    call.api_base = None;
    call.api_key = Some(&secret);
    let factory = Factory::new(Answer::json(IMAGE_ONE));
    let text = Text::replying(TEXT_EMPTY);
    let response = run(&call, None, &factory, &text).unwrap();
    assert_eq!(response.data.len(), 1);
    let requests = factory.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "/images/embeddings");
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        "Bearer async-key"
    );
    assert_eq!(
        requests[0].headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(factory.async_timeouts(), vec![Duration::from_secs(5)]);
    assert!(factory.sync_timeouts().is_empty());
    call.api_key = None;
    let keyless = Factory::new(Answer::json(IMAGE_ONE));
    run(&call, None, &keyless, &text).unwrap();
    assert_eq!(
        keyless.requests()[0].headers.get("authorization").unwrap(),
        "Bearer "
    );
}
