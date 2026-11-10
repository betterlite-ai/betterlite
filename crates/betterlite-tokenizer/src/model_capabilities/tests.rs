#![cfg(test)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::endpoints::{
    EndpointRegistry, EndpointSupport, PACKAGED_ASSET, RUNTIME_ASSET, RegistryAsset, parse,
};
use betterlite_models::lookup::capabilities::CapabilityFlag;
use betterlite_models::lookup::info::{ModelId, ModelInfoError, ModelQuery, PtuBounds};
use rstest::rstest;
use tokio::time::Instant;

use crate::count::Counter;
use crate::count::request::TextInput;
use crate::count::settings::Settings;
use crate::count::wire::{CustomTokenizerWire, ResponseTokens, TokenRequestWire};
use crate::engine::Engine;
use crate::engine::error::EngineError;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::hub::{Clock, HubDownloader};
use crate::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use crate::engine::selection::Selector;

use super::ModelCapabilities;

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

fn client() -> Arc<ProxiedClient> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    Arc::new(ProxiedClient::outbound(&policy).unwrap())
}

fn hub_settings(endpoint: &str, directory: &Path) -> HubSettings {
    HubSettings {
        endpoint: endpoint.to_owned(),
        offline: OfflineSetting::Online,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: directory.to_path_buf(),
        hub_cache: directory.to_path_buf(),
        token_path: directory.join("token"),
        stored_tokens_path: directory.join("stored_tokens"),
        transfer: TransferSetting::Standard,
    }
}

fn resolver(settings: &HubSettings) -> Arc<HubTokenResolver> {
    let oidc = OidcSettings {
        resource: None,
        id_token: None,
        github_actions: false,
        request_url: None,
        request_token: None,
        device_code_client_id: String::new(),
    };
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    Arc::new(HubTokenResolver::new(
        settings.clone(),
        oidc,
        client(),
        reader,
        clock,
    ))
}

fn system_clock() -> Arc<Clock> {
    Arc::new(Instant::now)
}

fn layer(directory: &Path) -> ModelCapabilities {
    let catalog = BundledCatalog::default().get();
    let registry = Arc::new(parse(PACKAGED_ASSET, RUNTIME_ASSET).unwrap());
    let settings = Settings {
        disabled: false,
        max_exact_chars: NonZeroUsize::new(4_000_000).unwrap(),
        chunk_chars: NonZeroUsize::new(4_000_000).unwrap(),
    };
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        EncodeLimits::default(),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(client(), None, Arc::clone(&service)));
    let hub = hub_settings("http://127.0.0.1:1", directory);
    let downloader = Arc::new(HubDownloader::new(
        Arc::new(hub.clone()),
        resolver(&hub),
        system_clock(),
    ));
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        downloader,
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    let counter = Arc::new(Counter::new(
        selector,
        fetcher,
        service,
        Arc::clone(&catalog),
        settings,
    ));
    ModelCapabilities::new(catalog, counter, registry)
}

#[derive(Clone, Copy, Debug)]
enum Check {
    KnownModel,
    MissingModel,
    CatalogSupportIsNotAnAdapter,
    UnsupportedRouteStaysUnsupported,
    ProviderMissingFromRegistry,
    TokenCountUsesTheCatalog,
    RuntimeRegistryIsPackaged,
    BedrockRoutingPrefixes,
}

#[rstest]
#[case::model_info_composes_catalog_metadata(Check::KnownModel)]
#[case::unknown_model_reports_unmapped(Check::MissingModel)]
#[case::catalog_support_is_not_an_installed_adapter(Check::CatalogSupportIsNotAnAdapter)]
#[case::listed_false_route_is_unsupported(Check::UnsupportedRouteStaysUnsupported)]
#[case::provider_missing_from_registry_has_no_routes(Check::ProviderMissingFromRegistry)]
#[case::token_count_uses_catalog_and_bundled_encoding(Check::TokenCountUsesTheCatalog)]
#[case::runtime_registry_is_the_packaged_backup(Check::RuntimeRegistryIsPackaged)]
#[case::bedrock_routing_prefixes_resolve_to_the_base_model(Check::BedrockRoutingPrefixes)]
#[tokio::test]
async fn model_capabilities_model_capability_tests_behavior(#[case] check: Check) {
    let directory = tempfile::tempdir().unwrap();
    let layer = layer(directory.path());
    match check {
        Check::KnownModel => known_model(&layer),
        Check::MissingModel => missing_model(&layer),
        Check::CatalogSupportIsNotAnAdapter => catalog_support_is_not_an_adapter(&layer),
        Check::UnsupportedRouteStaysUnsupported => unsupported_route(&layer),
        Check::ProviderMissingFromRegistry => provider_missing_from_registry(&layer),
        Check::TokenCountUsesTheCatalog => token_count(&layer).await,
        Check::RuntimeRegistryIsPackaged => runtime_registry_is_packaged(),
        Check::BedrockRoutingPrefixes => bedrock_routing_prefixes(&layer),
    }
}

fn known_model(layer: &ModelCapabilities) {
    let query = ModelQuery {
        model: "gpt-4o",
        provider: Some("openai"),
    };
    let info = layer
        .model_info(
            &query,
            Some(ModelId::Text("deployment-1")),
            &PtuBounds::default(),
        )
        .unwrap();
    assert_eq!(info.id, "deployment-1");
    assert_eq!(info.key, "gpt-4o");
    assert_eq!(info.provider.as_deref(), Some("openai"));
    assert_eq!(info.limits.max_input_tokens, Some(128_000));
    assert_eq!(info.capabilities.supports_vision, Some(true));
    assert_eq!(info.capabilities.get(CapabilityFlag::Vision), Some(true));
}

fn missing_model(layer: &ModelCapabilities) {
    let query = ModelQuery {
        model: "no-such-model",
        provider: Some("openai"),
    };
    let error = layer
        .model_info(&query, None, &PtuBounds::default())
        .unwrap_err();
    assert!(error.to_string().contains("This model isn't mapped yet."));
}

fn catalog_support_is_not_an_adapter(layer: &ModelCapabilities) {
    let query = ModelQuery {
        model: "gpt-4o",
        provider: Some("openai"),
    };
    let routes = layer.route_statuses(&query).unwrap();
    let chat = routes
        .iter()
        .find(|route| route.endpoint == "chat_completions")
        .unwrap();
    assert_eq!(chat.catalog_support, EndpointSupport::Supported);
    assert!(!chat.adapter_installed);
    assert!(routes.iter().all(|route| !route.adapter_installed));
}

fn unsupported_route(layer: &ModelCapabilities) {
    let query = ModelQuery {
        model: "gpt-4o",
        provider: Some("openai"),
    };
    let routes = layer.route_statuses(&query).unwrap();
    let rerank = routes
        .iter()
        .find(|route| route.endpoint == "rerank")
        .unwrap();
    assert_eq!(rerank.catalog_support, EndpointSupport::Unsupported);
    assert!(!rerank.adapter_installed);
}

fn provider_missing_from_registry(layer: &ModelCapabilities) {
    let query = ModelQuery {
        model: "aihubmix/agnes-2.5-flash",
        provider: None,
    };
    assert!(layer.route_statuses(&query).unwrap().is_empty());
}

async fn token_count(layer: &ModelCapabilities) {
    let wire = TokenRequestWire {
        model: "gpt-4o".to_owned(),
        custom_tokenizer: Some(CustomTokenizerWire::OpenAi),
        text: Some(TextInput::One("hello world".to_owned())),
        messages: None,
        count_response_tokens: ResponseTokens::default(),
        tools: None,
        tool_choice: None,
    };
    assert_eq!(layer.count_tokens(&wire).await.unwrap(), 2);
}

fn runtime_registry_is_packaged() {
    let registry: EndpointRegistry = parse(PACKAGED_ASSET, RUNTIME_ASSET).unwrap();
    assert_eq!(registry.asset, RegistryAsset::Packaged);
    assert_eq!(registry.providers.len(), 152);
}

fn bedrock_routing_prefixes(layer: &ModelCapabilities) {
    let base = "anthropic.claude-3-5-sonnet-20240620-v1:0";
    let routed = [
        format!("converse/{base}"),
        format!("apac.{base}"),
        format!("eu-west-1/{base}"),
    ];
    for model in &routed {
        let query = ModelQuery {
            model,
            provider: Some("bedrock"),
        };
        let info = layer
            .model_info(&query, None, &PtuBounds::default())
            .unwrap();
        assert_eq!(info.key, base, "{model}");
        assert_eq!(info.provider.as_deref(), Some("bedrock"));
        assert_eq!(info.model, *model);
    }
    let mantle = ModelQuery {
        model: "us-east-1/openai.gpt-oss-120b",
        provider: Some("bedrock_mantle"),
    };
    let info = layer
        .model_info(&mantle, None, &PtuBounds::default())
        .unwrap();
    assert_eq!(info.key, "bedrock_mantle/openai.gpt-oss-120b");
    let unregioned = ModelQuery {
        model: "not-a-region/openai.gpt-oss-120b",
        provider: Some("bedrock_mantle"),
    };
    let error = layer
        .model_info(&unregioned, None, &PtuBounds::default())
        .unwrap_err();
    assert!(matches!(error, ModelInfoError::Unmapped { .. }));
}
