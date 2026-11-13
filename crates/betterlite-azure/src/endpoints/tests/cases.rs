use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::azure_credentials::DEFAULT_SCOPE;
use secrecy::SecretString;

use super::support::{
    Cache, DEFAULT_API_BASE, DEFAULT_VERSION, Env, Inputs, KeyedCache, Run, Script, SourceCall,
    Sources, callable, get_client, initialize, initialize_result, revealed,
};
use crate::endpoints::cloudflare::{
    CloudflareClient, CloudflareOutcome, CloudflareParams, CloudflareRequest, ParamValue,
    init_cloudflare_gateway_client,
};
use crate::endpoints::dynamic::{DynamicVersionError, check_dynamic_azure_params};
use crate::endpoints::get_client::ClientCache as _;
use crate::endpoints::identity::client_identity;
use crate::endpoints::projection::{
    ClientInputs, ClientKind, ClientOutcome, ClientSource, OpenAiV1ClientInputs, SessionKind,
    SuppliedClient, V1Key,
};

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn default_api_version() {
    initializes_from_defaults();
    api_version_precedence();
    retry_precedence();
    timeout_forwarding();
    session_selection();
}

fn initializes_from_defaults() {
    let inputs = Inputs::default();
    let value = initialize(&inputs, &Run::default());
    assert_eq!(value.api_version, DEFAULT_VERSION);
    assert_eq!(value.max_retries, 2);
    assert!(value.timeout.is_none());
    assert!(!value.http_client.is_async);
    assert!(!value.http_client.shared);
    assert_eq!(
        value.endpoint.azure_endpoint.as_deref(),
        Some(DEFAULT_API_BASE)
    );
    assert!(value.endpoint.base_url.is_none());
    assert!(value.api_key.is_none());
    assert!(value.azure_ad_token.is_none());
    assert!(value.token_provider.is_none());
    assert!(inputs.sources.calls().is_empty());
}

fn api_version_precedence() {
    let mut inputs = Inputs::default();
    inputs
        .environment
        .0
        .insert("AZURE_API_VERSION".to_owned(), "2024-06-01".to_owned());
    assert_eq!(
        initialize(&inputs, &Run::default()).api_version,
        "2024-06-01"
    );
    inputs
        .environment
        .0
        .insert("AZURE_API_VERSION".to_owned(), String::new());
    assert_eq!(initialize(&inputs, &Run::default()).api_version, "");
    let explicit = Run::default().with_version(Some("2024-02-01"));
    assert_eq!(initialize(&inputs, &explicit).api_version, "2024-02-01");
}

fn retry_precedence() {
    let five = Inputs::default().with_max_retries(Some(5));
    assert_eq!(initialize(&five, &Run::default()).max_retries, 5);
    let zero = Inputs::default().with_max_retries(Some(0));
    assert_eq!(initialize(&zero, &Run::default()).max_retries, 0);
    let configured = Inputs::default().with_default_retries(7);
    assert_eq!(initialize(&configured, &Run::default()).max_retries, 7);
}

fn timeout_forwarding() {
    let unset = Inputs::default();
    assert!(initialize(&unset, &Run::default()).timeout.is_none());
    let set = Inputs::default().with_timeout(Some(1.5_f64));
    assert_eq!(initialize(&set, &Run::default()).timeout, Some(1.5_f64));
    let zero = Inputs::default().with_timeout(Some(0.0_f64));
    assert_eq!(initialize(&zero, &Run::default()).timeout, Some(0.0_f64));
}

fn session_selection() {
    let inputs = Inputs::default();
    let async_shared = Run::default().async_shared();
    let value = initialize(&inputs, &async_shared);
    assert!(value.http_client.is_async);
    assert!(value.http_client.shared);
    let deployment =
        Run::default().with_api_base("https://x.openai.azure.com/openai/deployments/gpt");
    let deployment_value = initialize(&inputs, &deployment);
    assert!(deployment_value.endpoint.azure_endpoint.is_none());
    assert_eq!(
        deployment_value.endpoint.base_url.as_deref(),
        Some("https://x.openai.azure.com/openai/deployments/gpt")
    );
}

fn cloudflare<'data>(
    params: &'data CloudflareParams,
    environment: &'data Env,
    sources: &'data Sources,
) -> CloudflareRequest<'data> {
    CloudflareRequest {
        api_base: "https://gateway.example/azure",
        model: "gpt-4o",
        api_version: "2024-02-01",
        max_retries: 2,
        timeout: 7.5_f64,
        params,
        api_key: None,
        azure_ad_token: None,
        token_provider: None,
        shared_session: false,
        is_async: false,
        client: None,
        environment,
        sources,
    }
}

fn gateway_client<'data>(request: &'data CloudflareRequest<'data>) -> CloudflareClient<'data> {
    match init_cloudflare_gateway_client(request).unwrap() {
        CloudflareOutcome::Constructed(client) => client,
        CloudflareOutcome::Supplied(_) => panic!("expected a constructed client"),
    }
}

fn cloudflare_defaults() {
    let params = CloudflareParams::default();
    let environment = Env::default();
    let sources = Sources::default();
    let request = cloudflare(&params, &environment, &sources);
    let client = gateway_client(&request);
    assert_eq!(client.base_url, "https://gateway.example/azure/gpt-4o");
    assert_eq!(client.api_version, "2024-02-01");
    assert_eq!(client.max_retries, 2);
    assert_eq!(client.timeout.to_bits(), 7.5_f64.to_bits());
    assert!(!client.shared_session);
    assert_eq!(client.kind, ClientKind::AzureOpenAi);
    assert!(client.api_key.is_none());
    assert!(client.azure_ad_token.is_none());
}

fn cloudflare_base_url_shape() {
    let params = CloudflareParams::default();
    let environment = Env::default();
    let sources = Sources::default();
    let mut slash = cloudflare(&params, &environment, &sources);
    slash.api_base = "https://gateway.example/azure/";
    assert_eq!(
        gateway_client(&slash).base_url,
        "https://gateway.example/azure/gpt-4o"
    );
    let mut empty = cloudflare(&params, &environment, &sources);
    empty.api_base = "";
    assert_eq!(gateway_client(&empty).base_url, "/gpt-4o");
}

fn cloudflare_api_key_wins() {
    let params = CloudflareParams::default();
    let environment = Env::default();
    let sources = Sources::default();
    let key = secret("fixture-api-key");
    let token = secret("fixture-ad-token");
    let mut request = cloudflare(&params, &environment, &sources);
    request.api_key = Some(&key);
    request.azure_ad_token = Some(&token);
    let client = gateway_client(&request);
    assert_eq!(
        client.api_key.as_ref().map(revealed),
        Some("fixture-api-key")
    );
    assert!(client.azure_ad_token.is_none());
}

fn cloudflare_plain_token() {
    let params = CloudflareParams::default();
    let environment = Env::default();
    let sources = Sources::default();
    let plain = secret("plain-token");
    let mut request = cloudflare(&params, &environment, &sources);
    request.azure_ad_token = Some(&plain);
    let client = gateway_client(&request);
    assert_eq!(
        client.azure_ad_token.as_ref().map(revealed),
        Some("plain-token")
    );
}

fn cloudflare_dict_get() {
    let mut environment = Env::default();
    environment
        .0
        .insert("AZURE_TENANT_ID".to_owned(), "env-tenant".to_owned());
    environment
        .0
        .insert("AZURE_CLIENT_ID".to_owned(), "env-client".to_owned());
    let params = CloudflareParams {
        tenant_id: Some(ParamValue::Null),
        client_id: None,
        azure_scope: None,
    };
    let sources = Sources::default();
    let oidc = secret("oidc/assertion");
    let mut request = cloudflare(&params, &environment, &sources);
    request.azure_ad_token = Some(&oidc);
    let client = gateway_client(&request);
    assert_eq!(
        client.azure_ad_token.as_ref().map(revealed),
        Some("oidc-token")
    );
    assert_eq!(
        sources.calls(),
        vec![SourceCall::Oidc {
            azure_ad_token: "oidc/assertion".to_owned(),
            client_id: Some("env-client".to_owned()),
            tenant_id: None,
            scope: Some(DEFAULT_SCOPE.to_owned()),
        }]
    );
}

fn cloudflare_oidc_scope() {
    let null_scope = CloudflareParams {
        tenant_id: Some(ParamValue::Value("explicit-tenant".to_owned())),
        client_id: Some(ParamValue::Value("explicit-client".to_owned())),
        azure_scope: Some(ParamValue::Null),
    };
    let rows = [
        (
            null_scope,
            None,
            (Some("explicit-client"), Some("explicit-tenant"), None),
        ),
        (
            CloudflareParams::default(),
            Some(String::new()),
            (None, None, Some(String::new())),
        ),
    ];
    for (params, scope_environment, (client_id, tenant_id, scope)) in rows {
        let mut environment = Env::default();
        if let Some(value) = scope_environment {
            environment.0.insert("AZURE_SCOPE".to_owned(), value);
        }
        let sources = Sources::default();
        let oidc = secret("oidc/assertion");
        let mut request = cloudflare(&params, &environment, &sources);
        request.azure_ad_token = Some(&oidc);
        gateway_client(&request);
        assert_eq!(
            sources.calls(),
            vec![SourceCall::Oidc {
                azure_ad_token: "oidc/assertion".to_owned(),
                client_id: client_id.map(str::to_owned),
                tenant_id: tenant_id.map(str::to_owned),
                scope,
            }]
        );
    }
}

fn cloudflare_async_and_shared() {
    let params = CloudflareParams::default();
    let environment = Env::default();
    let sources = Sources::default();
    let mut request = cloudflare(&params, &environment, &sources);
    request.is_async = true;
    request.shared_session = true;
    let client = gateway_client(&request);
    assert_eq!(client.kind, ClientKind::AsyncAzureOpenAi);
    assert!(client.shared_session);
}

fn cloudflare_supplied_client() {
    let params = CloudflareParams::default();
    let environment = Env::default();
    let sources = Sources::default();
    let supplied = SuppliedClient {
        kind: ClientKind::AsyncAzureOpenAi,
        custom_query: Some(BTreeMap::from([(
            "api-version".to_owned(),
            "2024-01-01".to_owned(),
        )])),
    };
    let mut request = cloudflare(&params, &environment, &sources);
    request.client = Some(&supplied);
    let CloudflareOutcome::Supplied(client) = init_cloudflare_gateway_client(&request).unwrap()
    else {
        panic!("a supplied client was rebuilt");
    };
    assert_eq!(client.kind, ClientKind::AsyncAzureOpenAi);
    assert_eq!(
        client.custom_query,
        Some(BTreeMap::from([(
            "api-version".to_owned(),
            "2024-01-01".to_owned()
        )]))
    );
}

pub(crate) fn cloudflare_base_model() {
    cloudflare_defaults();
    cloudflare_base_url_shape();
    cloudflare_api_key_wins();
    cloudflare_plain_token();
    cloudflare_dict_get();
    cloudflare_oidc_scope();
    cloudflare_async_and_shared();
    cloudflare_supplied_client();
}

fn identity_inputs() -> Inputs {
    let client_secret = secret("fixture-client-secret");
    let password = secret("fixture-password");
    let ad_token = secret("fixture-ad-token");
    Inputs::default()
        .with_tenant("tenant-a")
        .with_client_id("client-a")
        .with_client_secret(&client_secret)
        .with_username("user-a")
        .with_password(&password)
        .with_scope("scope-a")
        .with_ad_token(&ad_token)
}

fn constructed_identity() {
    let inputs = identity_inputs();
    let key = secret("fixture-api-key");
    let supplied_provider = callable(Script::Token("passed-token".to_owned()), 4242);
    let run = Run::for_client()
        .with_key(&key)
        .with_provider(&supplied_provider);
    let mut cache = Cache::empty();
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_eq!(outcome.source, ClientSource::Constructed);
    assert_eq!(outcome.kind, ClientKind::AzureOpenAi);
    assert!(!outcome.identity.is_async);
    assert_eq!(
        outcome.identity.hashed_api_key.as_deref(),
        Some("fdab98033749d02f02a57999bbe556414868bc3846375756506317674bace604")
    );
    assert_eq!(
        outcome.identity.hashed_ad_token.as_deref(),
        Some("0dcab650febbed3c396bb88c1b9a3933e8ea6db287d4fb58b148065292b841e3")
    );
    assert_eq!(
        outcome.identity.provider_identity,
        "provider_id=4242|tenant_id=tenant-a|client_id=client-a|client_secret=6cc747807fe29b510de22f59116767bf907738cdcc287a5587ec42bd5467d716|azure_username=user-a|azure_password=66c76b446bcb3002f4bebe35b74f7f7d4a3245190777e84273f02123a310d995|azure_scope=scope-a"
    );
    assert_eq!(cache.sets.len(), 1);
    assert_eq!(cache.sets[0].identity, outcome.identity);
    assert_eq!(cache.sets[0].kind, ClientKind::AzureOpenAi);
    assert!(cache.sets[0].owned);
    assert!(inputs.sources.calls().is_empty());
}

fn constructed_identity_owned_by_session() {
    let inputs = identity_inputs();
    let key = secret("fixture-api-key");
    let supplied_provider = callable(Script::Token("passed-token".to_owned()), 4242);
    let mut run = Run::for_client()
        .with_key(&key)
        .with_provider(&supplied_provider);
    run.session = SessionKind::Shared;
    let mut cache = Cache::empty();
    get_client(&inputs, &run, &mut cache).unwrap();
    assert!(!cache.sets[0].owned);
}

fn identities_stay_separate() {
    let first = identity_inputs();
    let second = Inputs::default().with_tenant("tenant-b");
    let key = secret("fixture-api-key");
    let mut cache = KeyedCache::empty();
    let run = Run::for_client().with_key(&key);
    let first_outcome = get_client(&first, &run, &mut cache).unwrap();
    let second_outcome = get_client(&second, &run, &mut cache).unwrap();
    assert_ne!(first_outcome.identity, second_outcome.identity);
    assert_eq!(
        cache.get(&first_outcome.identity),
        Some(ClientKind::AzureOpenAi)
    );
    assert_eq!(
        cache.get(&second_outcome.identity),
        Some(ClientKind::AzureOpenAi)
    );
    let other_key = secret("key-b");
    let third = client_identity(Some(&other_key), false, &Inputs::default().params, None);
    assert_eq!(cache.get(&third), None);
}

fn cached_clients_are_reused() {
    let inputs = identity_inputs();
    let key = secret("fixture-api-key");
    let mut cache = Cache::empty();
    cache.preloaded = Some(ClientKind::AzureOpenAi);
    let run = Run::for_client().with_version(Some("v1")).with_key(&key);
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_eq!(outcome.source, ClientSource::Cached);
    assert_eq!(outcome.kind, ClientKind::AzureOpenAi);
    assert!(cache.sets.is_empty());
    assert!(inputs.sources.calls().is_empty());
}

fn supplied_client_keeps_custom_query() {
    let inputs = identity_inputs();
    let explicit = SuppliedClient {
        kind: ClientKind::AsyncAzureOpenAi,
        custom_query: Some(BTreeMap::from([(
            "api-version".to_owned(),
            "2023-01-01".to_owned(),
        )])),
    };
    let mut cache = Cache::empty();
    let run = Run::for_client()
        .with_supplied(explicit)
        .with_version(Some("2024-02-01"));
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_eq!(outcome.source, ClientSource::Supplied);
    assert_eq!(
        outcome.custom_query,
        Some(BTreeMap::from([(
            "api-version".to_owned(),
            "2023-01-01".to_owned()
        )]))
    );
    assert!(!cache.sets[0].owned);
    assert!(inputs.sources.calls().is_empty());
}

fn supplied_openai_client_has_no_query() {
    let inputs = identity_inputs();
    let supplied = SuppliedClient {
        kind: ClientKind::OpenAi,
        custom_query: None,
    };
    let mut cache = Cache::empty();
    let run = Run::for_client().with_supplied(supplied);
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert!(outcome.custom_query.is_none());
}

pub(crate) fn separate_token_client_identity() {
    constructed_identity();
    constructed_identity_owned_by_session();
    identities_stay_separate();
    cached_clients_are_reused();
    supplied_client_keeps_custom_query();
    supplied_openai_client_has_no_query();
}

fn v1_versions_pick_openai() {
    let inputs = Inputs::default();
    for version in ["v1", "latest", "preview"] {
        for is_async in [false, true] {
            let mut run = Run::for_client().with_version(Some(version));
            run.is_async = is_async;
            let mut cache = Cache::empty();
            let outcome = get_client(&inputs, &run, &mut cache).unwrap();
            let expected = if is_async {
                ClientKind::AsyncOpenAi
            } else {
                ClientKind::OpenAi
            };
            assert_eq!(outcome.kind, expected, "{version}");
            assert_eq!(cache.sets[0].kind, expected, "{version}");
        }
    }
}

fn other_versions_pick_azure() {
    let inputs = Inputs::default();
    for version in ["V1", "Latest", "2024-02-01"] {
        let run = Run::for_client().with_version(Some(version));
        let mut cache = Cache::empty();
        let outcome = get_client(&inputs, &run, &mut cache).unwrap();
        assert_eq!(outcome.kind, ClientKind::AzureOpenAi, "{version}");
        assert_eq!(cache.sets[0].kind, ClientKind::AzureOpenAi, "{version}");
    }
}

fn environment_version_keeps_azure() {
    let mut inputs = Inputs::default();
    inputs
        .environment
        .0
        .insert("AZURE_API_VERSION".to_owned(), "v1".to_owned());
    let mut cache = Cache::empty();
    let run = Run::for_client().with_version(None);
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_eq!(outcome.kind, ClientKind::AzureOpenAi);
    match outcome.inputs.as_ref().unwrap() {
        ClientInputs::Azure(azure) => assert_eq!(azure.api_version, "v1"),
        ClientInputs::OpenAiV1(_) => panic!("an environment v1 picked the standard client"),
    }
}

fn cached_before_initialize() {
    let inputs = Inputs::default();
    let mut cache = Cache::empty();
    cache.preloaded = Some(ClientKind::AsyncOpenAi);
    let mut run = Run::for_client().with_version(Some("v1"));
    run.is_async = true;
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_eq!(outcome.source, ClientSource::Cached);
    assert_eq!(outcome.kind, ClientKind::AsyncOpenAi);
    assert!(cache.sets.is_empty());
}

fn ignored_cached_kind_constructs() {
    let inputs = identity_inputs();
    let key = secret("fixture-api-key");
    for (version, kind) in [
        (Some("2024-02-01"), ClientKind::AzureOpenAi),
        (Some("v1"), ClientKind::OpenAi),
    ] {
        let mut cache = Cache::empty();
        cache.preloaded = Some(ClientKind::Other);
        let run = Run::for_client().with_version(version).with_key(&key);
        let outcome = get_client(&inputs, &run, &mut cache).unwrap();
        assert_eq!(outcome.source, ClientSource::Constructed);
        assert_eq!(outcome.kind, kind);
    }
}

pub(crate) fn azure_sdk_client_mode() {
    v1_versions_pick_openai();
    other_versions_pick_azure();
    environment_version_keeps_azure();
    cached_before_initialize();
    ignored_cached_kind_constructs();
}

fn credentialed() -> Inputs {
    let client_secret = secret("client-secret");
    Inputs::default()
        .with_tenant("tenant")
        .with_client_id("client")
        .with_client_secret(&client_secret)
}

fn client_secret_provider() {
    let inputs = credentialed();
    let value = initialize(&inputs, &Run::default());
    assert!(value.token_provider.is_some());
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::ClientSecret {
            tenant_id: "tenant".to_owned(),
            client_id: "client".to_owned(),
            scope: DEFAULT_SCOPE.to_owned(),
        }]
    );
}

fn environment_credentials() {
    let mut inputs = Inputs::default();
    inputs
        .environment
        .0
        .insert("AZURE_TENANT_ID".to_owned(), "env-tenant".to_owned());
    inputs
        .environment
        .0
        .insert("AZURE_CLIENT_ID".to_owned(), "env-client".to_owned());
    inputs
        .environment
        .0
        .insert("AZURE_CLIENT_SECRET".to_owned(), "env-secret".to_owned());
    initialize(&inputs, &Run::default());
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::ClientSecret {
            tenant_id: "env-tenant".to_owned(),
            client_id: "env-client".to_owned(),
            scope: DEFAULT_SCOPE.to_owned(),
        }]
    );
}

fn explicit_empty_tenant_blocks() {
    let mut inputs = credentialed();
    inputs.params.auth.tenant_id = Some(String::new());
    inputs
        .environment
        .0
        .insert("AZURE_TENANT_ID".to_owned(), "env-tenant".to_owned());
    let value = initialize(&inputs, &Run::default());
    assert!(value.token_provider.is_none());
    assert!(inputs.sources.calls().is_empty());
}

fn api_key_blocks_client_secret() {
    let inputs = credentialed();
    let key = secret("api-key");
    let run = Run::default().with_key(&key);
    let value = initialize(&inputs, &run);
    assert!(value.token_provider.is_none());
    assert!(inputs.sources.calls().is_empty());
}

fn empty_api_key_allows_client_secret() {
    let inputs = credentialed();
    let key = secret("");
    let run = Run::default().with_key(&key);
    let value = initialize(&inputs, &run);
    assert!(value.token_provider.is_some());
    assert_eq!(inputs.sources.calls().len(), 1);
}

fn username_password_with_key() {
    let password = secret("password");
    let inputs = Inputs::default()
        .with_client_id("client")
        .with_username("user")
        .with_password(&password);
    let key = secret("api-key");
    let run = Run::default().with_key(&key);
    let value = initialize(&inputs, &run);
    assert!(value.token_provider.is_some());
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::UsernamePassword {
            client_id: "client".to_owned(),
            username: "user".to_owned(),
            scope: DEFAULT_SCOPE.to_owned(),
        }]
    );
}

fn client_secret_beats_username_password() {
    let password = secret("password");
    let inputs = credentialed()
        .with_username("user")
        .with_password(&password);
    initialize(&inputs, &Run::default());
    assert_eq!(inputs.sources.calls().len(), 1);
    assert!(matches!(
        inputs.sources.calls()[0],
        SourceCall::ClientSecret { .. }
    ));
}

fn refresh_provider() {
    let inputs = Inputs::default();
    let value = initialize(&inputs, &Run::default().refresh());
    assert!(value.token_provider.is_some());
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::Refresh {
            scope: DEFAULT_SCOPE.to_owned()
        }]
    );
}

fn refresh_keeps_empty_scope() {
    let mut inputs = Inputs::default();
    inputs
        .environment
        .0
        .insert("AZURE_SCOPE".to_owned(), String::new());
    initialize(&inputs, &Run::default().refresh());
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::Refresh {
            scope: String::new()
        }]
    );
}

fn refresh_swallows_value_error() {
    let mut inputs = Inputs::default();
    inputs.set_script(
        "refresh",
        Script::Raise {
            class: "ValueError".to_owned(),
            message: "no credential".to_owned(),
        },
    );
    let value = initialize(&inputs, &Run::default().refresh());
    assert!(value.token_provider.is_none());
}

fn refresh_propagates_other_errors() {
    let mut inputs = Inputs::default();
    inputs.set_script(
        "refresh",
        Script::Raise {
            class: "RuntimeError".to_owned(),
            message: "boom".to_owned(),
        },
    );
    let error = initialize_result(&inputs, &Run::default().refresh()).unwrap_err();
    assert_eq!(error.class(), "RuntimeError");
}

fn oidc_exchange_without_identity() {
    let key = secret("api-key");
    let inputs = Inputs::default().with_ad_token(&secret("oidc/assertion"));
    let run = Run::default().with_key(&key);
    let value = initialize(&inputs, &run);
    assert_eq!(
        value.azure_ad_token.as_ref().map(revealed),
        Some("oidc-token")
    );
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::Oidc {
            azure_ad_token: "oidc/assertion".to_owned(),
            client_id: None,
            tenant_id: None,
            scope: Some(DEFAULT_SCOPE.to_owned()),
        }]
    );
}

fn oidc_exchange_with_identity() {
    let inputs = Inputs::default()
        .with_tenant("tenant")
        .with_client_id("client")
        .with_ad_token(&secret("oidc/assertion"));
    initialize(&inputs, &Run::default());
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::Oidc {
            azure_ad_token: "oidc/assertion".to_owned(),
            client_id: Some("client".to_owned()),
            tenant_id: Some("tenant".to_owned()),
            scope: Some(DEFAULT_SCOPE.to_owned()),
        }]
    );
}

fn plain_token_still_refreshes() {
    let inputs = Inputs::default().with_ad_token(&secret("plain-token"));
    let value = initialize(&inputs, &Run::default().refresh());
    assert_eq!(
        value.azure_ad_token.as_ref().map(revealed),
        Some("plain-token")
    );
    assert_eq!(
        inputs.sources.calls(),
        vec![SourceCall::Refresh {
            scope: DEFAULT_SCOPE.to_owned()
        }]
    );
}

fn passed_provider_is_used() {
    let passed = callable(Script::Token("passed-token".to_owned()), 9);
    let inputs = credentialed();
    let run = Run::default().with_provider(&passed);
    let value = initialize(&inputs, &run);
    assert!(inputs.sources.calls().is_empty());
    assert!(Arc::ptr_eq(
        &value.token_provider.as_ref().unwrap().0,
        &passed.provider
    ));
}

pub(crate) fn azure_initializer_credentials() {
    client_secret_provider();
    environment_credentials();
    explicit_empty_tenant_blocks();
    api_key_blocks_client_secret();
    empty_api_key_allows_client_secret();
    username_password_with_key();
    client_secret_beats_username_password();
    refresh_provider();
    refresh_keeps_empty_scope();
    refresh_swallows_value_error();
    refresh_propagates_other_errors();
    oidc_exchange_without_identity();
    oidc_exchange_with_identity();
    plain_token_still_refreshes();
    passed_provider_is_used();
}

fn v1_inputs<'data>(value: &'data ClientOutcome<'data>) -> &'data OpenAiV1ClientInputs<'data> {
    match value.inputs.as_ref().unwrap() {
        ClientInputs::OpenAiV1(inputs) => inputs,
        ClientInputs::Azure(_) => panic!("a v1 version picked the Azure client"),
    }
}

fn v1_key_precedence() {
    let key = secret("fixture-api-key");
    let inputs = Inputs::default().with_ad_token(&secret("fixture-ad-token"));
    let mut cache = Cache::empty();
    let run = Run::for_client().with_version(Some("v1")).with_key(&key);
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_eq!(outcome.kind, ClientKind::OpenAi);
    match v1_inputs(&outcome).key.as_ref().unwrap() {
        V1Key::Static(value) => assert_eq!(revealed(value), "fixture-api-key"),
        V1Key::Provider { .. } => panic!("the api key lost to the token"),
    }
    assert_eq!(
        v1_inputs(&outcome).base_url.as_deref(),
        Some("https://example.openai.azure.com/openai/v1/")
    );
    assert!(v1_inputs(&outcome).timeout.is_none());
    assert_eq!(v1_inputs(&outcome).max_retries, 2);
    assert!(!v1_inputs(&outcome).http_client.is_async);
}

fn v1_provider_key() {
    let inputs = Inputs::default().with_ad_token(&secret("fixture-ad-token"));
    let supplied = callable(Script::Token("passed-token".to_owned()), 1);
    let mut cache = Cache::empty();
    let run = Run::for_client()
        .with_version(Some("v1"))
        .with_provider(&supplied);
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    match v1_inputs(&outcome).key.as_ref().unwrap() {
        V1Key::Provider { offload, .. } => assert!(!offload),
        V1Key::Static(_) => panic!("the provider lost to the token"),
    }
    let mut async_cache = Cache::empty();
    let mut async_run = Run::for_client()
        .with_version(Some("latest"))
        .with_provider(&supplied);
    async_run.is_async = true;
    let async_outcome = get_client(&inputs, &async_run, &mut async_cache).unwrap();
    assert_eq!(async_outcome.kind, ClientKind::AsyncOpenAi);
    match v1_inputs(&async_outcome).key.as_ref().unwrap() {
        V1Key::Provider { offload, .. } => assert!(*offload),
        V1Key::Static(_) => panic!("the async provider was not offloaded"),
    }
}

fn v1_token_key() {
    let inputs = Inputs::default().with_ad_token(&secret("fixture-ad-token"));
    let mut cache = Cache::empty();
    let run = Run::for_client().with_version(Some("preview"));
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    match v1_inputs(&outcome).key.as_ref().unwrap() {
        V1Key::Static(value) => assert_eq!(revealed(value), "fixture-ad-token"),
        V1Key::Provider { .. } => panic!("the token lost to a provider"),
    }
}

fn v1_absent_key() {
    let inputs = Inputs::default();
    let mut cache = Cache::empty();
    let run = Run::for_client().with_version(Some("v1"));
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert!(v1_inputs(&outcome).key.is_none());
}

fn v1_forwards_parameters() {
    let inputs = Inputs::default()
        .with_timeout(Some(2.5_f64))
        .with_max_retries(Some(4));
    let mut cache = Cache::empty();
    let run = Run::for_client()
        .with_version(Some("v1"))
        .with_api_base("https://x.openai.azure.com/openai/deployments/gpt")
        .async_shared();
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    let v1 = v1_inputs(&outcome);
    assert_eq!(
        v1.base_url.as_deref(),
        Some("https://x.openai.azure.com/openai/deployments/gpt/openai/v1/")
    );
    assert_eq!(v1.timeout, Some(2.5_f64));
    assert_eq!(v1.max_retries, 4);
    assert!(v1.http_client.is_async);
    assert!(v1.http_client.shared);
    assert!(!cache.sets[0].owned);
}

fn v1_base_url_is_literal() {
    let inputs = Inputs::default();
    let mut cache = Cache::empty();
    let run = Run::for_client()
        .with_version(Some("v1"))
        .with_api_base("https://x.openai.azure.com/");
    let outcome = get_client(&inputs, &run, &mut cache).unwrap();
    assert_eq!(
        v1_inputs(&outcome).base_url.as_deref(),
        Some("https://x.openai.azure.com//openai/v1/")
    );
}

pub(crate) fn azure_v1_client_projection() {
    v1_key_precedence();
    v1_provider_key();
    v1_token_key();
    v1_absent_key();
    v1_forwards_parameters();
    v1_base_url_is_literal();
}

pub(crate) fn dynamic_api_version() {
    let query = BTreeMap::from([("api-version".to_owned(), "2024-02-01".to_owned())]);
    let empty = BTreeMap::new();
    assert_eq!(check_dynamic_azure_params(None, None), Ok(true));
    assert_eq!(check_dynamic_azure_params(Some("v1"), None), Ok(true));
    assert_eq!(check_dynamic_azure_params(None, Some(&query)), Ok(false));
    assert_eq!(
        check_dynamic_azure_params(Some("2024-02-01"), Some(&query)),
        Ok(false)
    );
    assert_eq!(
        check_dynamic_azure_params(Some("v1"), Some(&query)),
        Ok(true)
    );
    assert_eq!(
        check_dynamic_azure_params(Some("v1"), Some(&empty)),
        Err(DynamicVersionError::MissingApiVersion)
    );
    assert_eq!(check_dynamic_azure_params(None, Some(&empty)), Ok(false));
}
