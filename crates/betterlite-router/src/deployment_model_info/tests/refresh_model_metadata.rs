use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::model_registration::JsonObject;
use tokio::net::TcpListener;
use tokio::time::timeout;

use betterlite_openai::compatible_models::TokenLimits;

use super::fixtures::{
    Counting, EXCHANGE_TIMEOUT, Events, Recorder, Resolver, accept_connections, answer_connections,
    answer_replacing, authorization, capture, catalog, client, clock, credentials, dispatcher,
    handle, limits, object, record,
};
use crate::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_model_info::cache::{DISCOVERY_TTL_SECONDS, DiscoveryCache};
use crate::deployment_model_info::refresh::{RefreshContext, refresh_model_info};

const ONE: &[u8] = br#"{"data":[{"id":"m0","context_length":4096}]}"#;
const TWO: &[u8] =
    br#"{"data":[{"id":"m0","context_length":4096},{"id":"m1","context_length":4096}]}"#;
const EIGHT: &[u8] = br#"{"data":[{"id":"m0","context_length":4096},{"id":"m1","context_length":4096},{"id":"m2","context_length":4096},{"id":"m3","context_length":4096},{"id":"m4","context_length":4096},{"id":"m5","context_length":4096},{"id":"m6","context_length":4096},{"id":"m7","context_length":4096}]}"#;
const ALL_MODELS: &str = "Could not refresh deployment model info";

struct Rig {
    handle: CatalogHandle,
    discovery: DiscoveryCache,
    credentials: CredentialSnapshot,
    invalidation: Counting,
    recorder: Recorder,
    resolver: Resolver,
    client: ProxiedClient,
}

impl Rig {
    fn new(engine: Arc<DeploymentCatalog>, discovery: DiscoveryCache) -> Self {
        Self::with_credentials(engine, discovery, credentials(&[]), Resolver::new(&[]))
    }

    fn with_credentials(
        engine: Arc<DeploymentCatalog>,
        discovery: DiscoveryCache,
        credential_list: CredentialSnapshot,
        resolver: Resolver,
    ) -> Self {
        Self {
            handle: handle(&engine),
            discovery,
            credentials: credential_list,
            invalidation: Counting::default(),
            recorder: Recorder::default(),
            resolver,
            client: client(),
        }
    }

    async fn run(&self) {
        let resolution = self.resolver.context();
        let context = RefreshContext {
            catalogs: &self.handle,
            discovery: &self.discovery,
            credentials: &self.credentials,
            resolution: &resolution,
            invalidation: &self.invalidation,
            client: &self.client,
            model_list_cache: &self.recorder,
        };
        refresh_model_info(&context).await;
    }

    fn cached(&self, model_id: &str) -> Option<TokenLimits> {
        self.discovery.get(model_id).map(|entry| entry.limits)
    }

    fn entries(&self) -> Vec<String> {
        self.discovery.report().entries
    }
}

async fn endpoint() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    (listener, format!("http://{address}"))
}

fn params(model: &str, provider: &str, api_base: &str) -> JsonObject {
    object(&format!(
        r#"{{"model":"{model}","custom_llm_provider":"{provider}","api_base":"{api_base}"}}"#
    ))
}

fn params_with(model: &str, provider: &str, api_base: &str, extra: &str) -> JsonObject {
    let mut built = params(model, provider, api_base);
    built.extend(object(extra));
    built
}

fn row(model_name: &str, id: &str, params: JsonObject) -> DeploymentSpec {
    let info = object(&format!(r#"{{"id":"{id}"}}"#));
    DeploymentSpec::new(
        model_name.to_owned(),
        params,
        info,
        JsonObject::new(),
        || "generated".to_owned(),
    )
    .unwrap()
}

async fn serve_n(listener: &TcpListener, count: usize, body: &[u8]) -> Vec<String> {
    let streams = timeout(EXCHANGE_TIMEOUT, accept_connections(listener, count))
        .await
        .expect("every expected request reaches the server at once");
    answer_connections(streams, body).await
}

fn full_limits(context: u64) -> Option<TokenLimits> {
    Some(limits(Some(context), Some(context), Some(context)))
}

#[tokio::test]
async fn snapshot_eight_workers() {
    let (listener, base) = endpoint().await;
    let specs = (0..8)
        .map(|index| {
            row(
                "group",
                &format!("id{index}"),
                params(&format!("m{index}"), "openai", &format!("{base}/d{index}")),
            )
        })
        .collect::<Vec<_>>();
    let engine = catalog(specs);
    let rig = Rig::new(Arc::clone(&engine), DiscoveryCache::for_deployments(8));
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 8, EIGHT));
    assert_eq!(requests.len(), 8);
    let cached = (0..8)
        .map(|index| rig.cached(&format!("id{index}")))
        .collect::<Vec<_>>();
    assert!(cached.iter().all(|entry| *entry == full_limits(4096)));
    assert_eq!(rig.entries().len(), 8);
    assert_eq!(rig.invalidation.count(), 9);
}

#[tokio::test]
async fn invalid_one_does_not_stop_others() {
    let (listener, base) = endpoint().await;
    let broken = row(
        "group",
        "bad-id",
        object(r#"{"model":5,"api_base":"http://127.0.0.1:1"}"#),
    );
    let good = row(
        "group",
        "good-id",
        params("m0", "openai", &format!("{base}/good")),
    );
    let rig = Rig::new(
        catalog(vec![broken, good]),
        DiscoveryCache::for_deployments(2),
    );
    let events = Events::default();
    let _guard = tracing::subscriber::set_default(capture(events.clone()));
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 1, ONE));
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("/good/v1/models"));
    assert_eq!(rig.cached("good-id"), full_limits(4096));
    assert!(rig.cached("bad-id").is_none());
    assert_eq!(events.at("DEBUG"), vec![ALL_MODELS.to_owned()]);
    assert!(events.at("WARN").is_empty());
    assert!(events.at("ERROR").is_empty());
}

#[tokio::test]
async fn debug_not_warning_on_failure() {
    let broken_model = row(
        "group",
        "bad-id",
        object(r#"{"model":5,"api_base":"http://127.0.0.1:1"}"#),
    );
    let broken_key = row(
        "group",
        "bad-key",
        object(r#"{"model":"openai/m0","api_base":"http://127.0.0.1:1","api_key":5}"#),
    );
    let rig = Rig::new(
        catalog(vec![broken_model, broken_key]),
        DiscoveryCache::for_deployments(2),
    );
    let events = Events::default();
    let _guard = tracing::subscriber::set_default(capture(events.clone()));
    rig.run().await;
    assert_eq!(
        events.at("DEBUG"),
        vec![ALL_MODELS.to_owned(), ALL_MODELS.to_owned()]
    );
    assert!(events.at("WARN").is_empty());
    assert!(events.at("ERROR").is_empty());
    assert!(rig.entries().is_empty());
    assert_eq!(rig.invalidation.count(), 1);
}

#[tokio::test]
async fn empty_refresh_invalidates() {
    let rig = Rig::new(catalog(vec![]), DiscoveryCache::for_deployments(1));
    rig.run().await;
    assert_eq!(rig.invalidation.count(), 1);
    assert!(rig.recorder.stored().is_empty());
    assert!(rig.entries().is_empty());
}

#[tokio::test]
async fn credential_overlay_before_resolution() {
    let (listener, base) = endpoint().await;
    let named = row(
        "group",
        "id0",
        object(r#"{"model":"openai/m0","litellm_credential_name":"cred"}"#),
    );
    let values = format!(r#"{{"api_base":"{base}/cred","api_key":"sk-cred"}}"#);
    let named_credentials = credentials(&[("cred", values.as_str())]);
    let rig = Rig::with_credentials(
        catalog(vec![named]),
        DiscoveryCache::for_deployments(1),
        named_credentials,
        Resolver::new(&[]),
    );
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 1, ONE));
    assert_eq!(requests.len(), 1);
    let request = requests[0].to_lowercase();
    assert!(request.contains("/cred/v1/models"), "{request}");
    assert!(
        request.contains("authorization: bearer sk-cred"),
        "{request}"
    );
    assert_eq!(rig.cached("id0"), full_limits(4096));
}

#[tokio::test]
async fn four_provider_allowlist() {
    let (listener, base) = endpoint().await;
    let body = br#"{"data":[{"id":"hv","context_length":10},{"id":"oai","context_length":20},{"id":"tc","context_length":30},{"id":"ol","context_length":40}]}"#;
    let specs = vec![
        row(
            "group",
            "hosted",
            params("hv", "hosted_vllm", &format!("{base}/hv")),
        ),
        row(
            "group",
            "openai",
            params("oai", "openai", &format!("{base}/oai")),
        ),
        row(
            "group",
            "text",
            params("tc", "text-completion-openai", &format!("{base}/tc")),
        ),
        row(
            "group",
            "like",
            params("ol", "openai_like", &format!("{base}/ol")),
        ),
        row(
            "group",
            "anthropic",
            params("claude", "anthropic", &format!("{base}/an")),
        ),
    ];
    let rig = Rig::new(catalog(specs), DiscoveryCache::for_deployments(5));
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 4, body));
    assert_eq!(requests.len(), 4);
    assert_eq!(rig.cached("hosted"), full_limits(10));
    assert_eq!(rig.cached("openai"), full_limits(20));
    assert_eq!(rig.cached("text"), full_limits(30));
    assert_eq!(rig.cached("like"), full_limits(40));
    assert!(rig.cached("anthropic").is_none());
}

#[tokio::test]
async fn absent_base_wildcard_and_clientside_skips() {
    let specs = vec![
        row("group", "no-base", object(r#"{"model":"openai/m0"}"#)),
        row(
            "group",
            "wild",
            object(r#"{"model":"openai/*","api_base":"http://127.0.0.1:1/wild"}"#),
        ),
        row(
            "group",
            "clientside",
            params_with(
                "m1",
                "openai",
                "http://127.0.0.1:1/clientside",
                r#"{"use_clientside_credentials":true}"#,
            ),
        ),
        row(
            "group",
            "falsy",
            params_with(
                "m2",
                "openai",
                "http://127.0.0.1:1/falsy",
                r#"{"use_clientside_credentials":false}"#,
            ),
        ),
    ];
    let rig = Rig::new(catalog(specs), DiscoveryCache::for_deployments(4));
    rig.run().await;
    assert_eq!(rig.recorder.stored().len(), 1);
    assert!(rig.entries().is_empty());
    assert_eq!(rig.invalidation.count(), 1);
}

#[tokio::test]
async fn dynamic_key_fallback() {
    let (listener, base) = endpoint().await;
    let from_environment = row(
        "group",
        "env-key",
        params("m0", "hosted_vllm", &format!("{base}/a")),
    );
    let from_params = row(
        "group",
        "param-key",
        params_with(
            "m1",
            "hosted_vllm",
            &format!("{base}/b"),
            r#"{"api_key":"sk-param"}"#,
        ),
    );
    let empty_key = row(
        "group",
        "empty-key",
        params_with(
            "m2",
            "hosted_vllm",
            &format!("{base}/c"),
            r#"{"api_key":""}"#,
        ),
    );
    // The "openai" model group name turns off provider defaults, so the resolution keeps no key at all.
    let prefixed = row(
        "group",
        "prefix-key",
        params_with(
            "openai/m3",
            "openai",
            &format!("{base}/d"),
            r#"{"api_key":"sk-prefix"}"#,
        ),
    );
    let prefixed_empty = row(
        "group",
        "prefix-empty",
        params_with(
            "openai/m4",
            "openai",
            &format!("{base}/e"),
            r#"{"api_key":""}"#,
        ),
    );
    let rig = Rig::with_credentials(
        catalog(vec![
            from_environment,
            from_params,
            empty_key,
            prefixed,
            prefixed_empty,
        ]),
        DiscoveryCache::for_deployments(5),
        credentials(&[]),
        Resolver::with_known(&[("HOSTED_VLLM_API_KEY", "sk-env")], &["openai"]),
    );
    let body = br#"{"data":[{"id":"m0","context_length":4096},{"id":"m1","context_length":4096},{"id":"m2","context_length":4096},{"id":"m3","context_length":4096},{"id":"m4","context_length":4096}]}"#;
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 5, body));
    assert_eq!(requests.len(), 5);
    assert_eq!(
        authorization(&requests, "/a/v1/models"),
        Some("bearer sk-env".to_owned())
    );
    assert_eq!(
        authorization(&requests, "/b/v1/models"),
        Some("bearer sk-param".to_owned())
    );
    assert_eq!(
        authorization(&requests, "/c/v1/models"),
        Some("bearer".to_owned())
    );
    assert_eq!(
        authorization(&requests, "/d/v1/models"),
        Some("bearer sk-prefix".to_owned())
    );
    assert_eq!(authorization(&requests, "/e/v1/models"), None);
    assert_eq!(rig.cached("env-key"), full_limits(4096));
    assert_eq!(rig.cached("param-key"), full_limits(4096));
    assert_eq!(rig.cached("empty-key"), full_limits(4096));
    assert_eq!(rig.cached("prefix-key"), full_limits(4096));
    assert_eq!(rig.cached("prefix-empty"), full_limits(4096));
}

#[tokio::test]
async fn extra_headers_then_headers() {
    let (listener, base) = endpoint().await;
    let with_extra = row(
        "group",
        "extra",
        params_with(
            "m0",
            "openai",
            &format!("{base}/a"),
            r#"{"extra_headers":{"X-Extra":"one"},"headers":{"X-Base":"two"}}"#,
        ),
    );
    let headers_only = row(
        "group",
        "headers",
        params_with(
            "m1",
            "openai",
            &format!("{base}/b"),
            r#"{"headers":{"X-Base":"two"}}"#,
        ),
    );
    let rig = Rig::new(
        catalog(vec![with_extra, headers_only]),
        DiscoveryCache::for_deployments(2),
    );
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 2, TWO));
    assert_eq!(requests.len(), 2);
    let extra = requests
        .iter()
        .find(|request| request.contains("/a/v1/models"))
        .map(|request| request.to_lowercase())
        .unwrap();
    assert!(extra.contains("x-extra: one"), "{extra}");
    assert!(!extra.contains("x-base"), "{extra}");
    let headers = requests
        .iter()
        .find(|request| request.contains("/b/v1/models"))
        .map(|request| request.to_lowercase())
        .unwrap();
    assert!(headers.contains("x-base: two"), "{headers}");
}

#[tokio::test]
async fn empty_header_containers_fall_through() {
    let (listener, base) = endpoint().await;
    let empty_object = row(
        "group",
        "object",
        params_with(
            "m0",
            "openai",
            &format!("{base}/a"),
            r#"{"extra_headers":{},"headers":{"X-Base":"one"}}"#,
        ),
    );
    let empty_list = row(
        "group",
        "list",
        params_with(
            "m1",
            "openai",
            &format!("{base}/b"),
            r#"{"extra_headers":[],"headers":{"X-Base":"two"}}"#,
        ),
    );
    let rig = Rig::new(
        catalog(vec![empty_object, empty_list]),
        DiscoveryCache::for_deployments(2),
    );
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 2, TWO));
    assert_eq!(requests.len(), 2);
    let object = requests
        .iter()
        .find(|request| request.contains("/a/v1/models"))
        .map(|request| request.to_lowercase())
        .unwrap();
    assert!(object.contains("x-base: one"), "{object}");
    let list = requests
        .iter()
        .find(|request| request.contains("/b/v1/models"))
        .map(|request| request.to_lowercase())
        .unwrap();
    assert!(list.contains("x-base: two"), "{list}");
}

#[tokio::test]
async fn lowercase_explicit_authorization_wins() {
    let (listener, base) = endpoint().await;
    let deployment = row(
        "group",
        "id0",
        params_with(
            "m0",
            "openai",
            &format!("{base}/a"),
            r#"{"api_key":"sk-param","extra_headers":{"Authorization":"Bearer custom"}}"#,
        ),
    );
    let rig = Rig::new(
        catalog(vec![deployment]),
        DiscoveryCache::for_deployments(1),
    );
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 1, ONE));
    let request = requests[0].to_lowercase();
    assert!(
        request.contains("authorization: bearer custom"),
        "{request}"
    );
    assert!(!request.contains("bearer sk-param"), "{request}");
}

#[tokio::test]
async fn invalid_header_values() {
    let number = row(
        "group",
        "number",
        params_with(
            "m0",
            "openai",
            "http://127.0.0.1:1/a",
            r#"{"headers":{"X-Num":5}}"#,
        ),
    );
    let broken = row(
        "group",
        "broken",
        params_with(
            "m1",
            "openai",
            "http://127.0.0.1:1/b",
            r#"{"extra_headers":["a"]}"#,
        ),
    );
    let rig = Rig::new(
        catalog(vec![number, broken]),
        DiscoveryCache::for_deployments(2),
    );
    let events = Events::default();
    let _guard = tracing::subscriber::set_default(capture(events.clone()));
    rig.run().await;
    assert!(rig.recorder.stored().is_empty());
    assert!(rig.entries().is_empty());
    assert_eq!(
        events.at("DEBUG"),
        vec![ALL_MODELS.to_owned(), ALL_MODELS.to_owned()]
    );
}

#[tokio::test]
async fn empty_discovery_no_write() {
    let (listener, base) = endpoint().await;
    let deployment = row("group", "id0", params("m0", "openai", &format!("{base}/a")));
    let rig = Rig::new(
        catalog(vec![deployment]),
        DiscoveryCache::for_deployments(1),
    );
    let empty = br#"{"data":[]}"#;
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 1, empty));
    assert_eq!(requests.len(), 1);
    assert_eq!(rig.recorder.stored().len(), 1);
    assert!(rig.entries().is_empty());
    assert_eq!(rig.invalidation.count(), 1);
}

#[tokio::test]
async fn missing_id_no_write() {
    let (listener, base) = endpoint().await;
    let deployment = DeploymentSpec {
        model_name: "group".to_owned(),
        litellm_params: params("m0", "openai", &format!("{base}/a")),
        model_info: object(r#"{"owned_by":"someone"}"#),
        extra: JsonObject::new(),
    };
    let rig = Rig::new(
        catalog(vec![deployment]),
        DiscoveryCache::for_deployments(1),
    );
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 1, ONE));
    assert_eq!(requests.len(), 1);
    assert_eq!(rig.recorder.stored().len(), 1);
    assert!(rig.entries().is_empty());
    assert_eq!(rig.invalidation.count(), 1);
}

#[tokio::test]
async fn replacement_during_request_no_write() {
    let (listener, base) = endpoint().await;
    let model_params = params("m0", "openai", &format!("{base}/a"));
    let engine = catalog(vec![row("group", "id0", model_params.clone())]);
    let replacement = catalog(vec![row("group", "id0", model_params)]);
    let rig = Rig::new(Arc::clone(&engine), DiscoveryCache::for_deployments(1));
    let ((), request) = tokio::join!(rig.run(), async {
        let (stream, _) = listener.accept().await.unwrap();
        answer_replacing(stream, &rig.handle, Arc::clone(&replacement), ONE).await
    });
    assert!(request.contains("/a/v1/models"));
    assert!(rig.entries().is_empty());
    assert_eq!(rig.invalidation.count(), 1);
}

#[tokio::test]
async fn resize_delete_set_invalidate_order() {
    let (listener, base) = endpoint().await;
    let clock_handle = dispatcher(500.0);
    let discovery = DiscoveryCache::new(1, DISCOVERY_TTL_SECONDS, clock(&clock_handle));
    let engine = catalog(vec![
        row("group", "id0", params("m0", "openai", &format!("{base}/a"))),
        row("group", "id1", params("m1", "openai", &format!("{base}/b"))),
    ]);
    discovery.store("id0", record(&engine, 0, limits(Some(9), Some(9), Some(9))));
    *clock_handle.lock().unwrap() = 1000.0;
    let rig = Rig::new(Arc::clone(&engine), discovery);
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 2, TWO));
    assert_eq!(requests.len(), 2);
    let report = rig.discovery.report();
    assert_eq!(report.capacity, 2);
    assert_eq!(report.entries, vec!["id0".to_owned(), "id1".to_owned()]);
    assert_eq!(
        report.expirations,
        vec![("id0".to_owned(), 1600.0), ("id1".to_owned(), 1600.0)]
    );
    assert_eq!(rig.cached("id0"), full_limits(4096));
    assert_eq!(rig.invalidation.count(), 3);
}

#[tokio::test]
async fn discovery_ttl_six_hundred() {
    let (listener, base) = endpoint().await;
    let clock_handle = dispatcher(1000.0);
    let discovery = DiscoveryCache::new(1, DISCOVERY_TTL_SECONDS, clock(&clock_handle));
    let engine = catalog(vec![row(
        "group",
        "id0",
        params("m0", "openai", &format!("{base}/a")),
    )]);
    let rig = Rig::new(Arc::clone(&engine), discovery);
    let ((), requests) = tokio::join!(rig.run(), serve_n(&listener, 1, ONE));
    assert_eq!(requests.len(), 1);
    assert_eq!(
        rig.discovery.report().expirations,
        vec![("id0".to_owned(), 1600.0)]
    );
    *clock_handle.lock().unwrap() = 1600.0;
    assert!(rig.discovery.get("id0").is_some());
    *clock_handle.lock().unwrap() = 1600.5;
    assert!(rig.discovery.get("id0").is_none());
}
