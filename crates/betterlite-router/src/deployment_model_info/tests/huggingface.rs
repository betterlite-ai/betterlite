use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::HelperSources;
use betterlite_models::full_model_metadata::cache::ModelInfoCache;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::hf_config::{HF_CONFIG_BASE_URL, HfConfigClient};
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use super::fixtures::{
    Environment, FakeHf, NoDynamic, Resolver, catalog, object, registry, snapshot,
};
use crate::deployment_model_info::cache::DiscoveryCache;
use crate::deployment_model_info::metadata::{HelperModelMetadata, RouterMetadata};
use crate::deployment_model_info::router_info::{SuppliedDeployment, get_router_model_info};

async fn merged(body: Option<&'static str>) -> Option<JsonObject> {
    let environment = Environment::new("{}", &[]);
    let harness = environment.harness();
    let metadata_environment = harness.metadata();
    let hf = HfConfigClient {
        client: FakeHf { body },
        base_url: HF_CONFIG_BASE_URL.to_owned(),
    };
    let sources = HelperSources {
        environment: &metadata_environment,
        dynamic: &NoDynamic,
        hf: &hf,
    };
    let cache = ModelInfoCache::new(128);
    let source = HelperModelMetadata {
        sources: &sources,
        cache: &cache,
    };
    let engine = catalog(Vec::new());
    let snapshot = snapshot(&engine);
    let discovery = DiscoveryCache::for_deployments(1);
    let registry = registry("{}");
    let resolver = Resolver::new(&[]);
    let resolution = resolver.context();
    let metadata = RouterMetadata {
        snapshot: &snapshot,
        discovery: &discovery,
        registry: &registry,
        resolution: &resolution,
        source: &source,
    };
    let params = object(r#"{"model":"org/model","custom_llm_provider":"huggingface"}"#);
    let model_info = object(r#"{"id":"one"}"#);
    let deployment = SuppliedDeployment {
        model_info: &model_info,
        litellm_params: Some(&params),
    };
    let mut keys = ModelCostKeys::default();
    get_router_model_info(&metadata, Some(deployment), "group", None, &mut keys)
        .await
        .unwrap()
}

#[tokio::test]
async fn native_huggingface_metadata_fetches_the_config() {
    let info = merged(Some(r#"{"max_position_embeddings": 4096}"#))
        .await
        .unwrap();
    assert_eq!(
        info.get("max_tokens"),
        Some(&Json::Number(Number::from(4096)))
    );
    assert_eq!(
        info.get("litellm_provider"),
        Some(&Json::Text("huggingface".to_owned()))
    );
    let omitted = merged(Some("{}")).await.unwrap();
    assert_eq!(omitted.get("max_tokens"), Some(&Json::Null));
    let failed = merged(None).await.unwrap();
    assert_eq!(failed.get("max_tokens"), Some(&Json::Null));
}
