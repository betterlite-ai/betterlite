use betterlite_context::bedrock_model::regions::AmazonBedrockGlobalConfig;
use betterlite_models::provider_defaults_a::policies::{
    MantleEndpoint, ModelSets, ProviderPolicies,
};

use super::PassthroughCallArgs;

// get_llm_provider's bedrock_mantle branch: B2207's endpoint inputs, from the call's model, base and key
const PROVIDER_PREFIX: &str = "bedrock_mantle/";
const DEFAULT_REGION: &str = "us-east-1";

pub(super) fn policies(args: &PassthroughCallArgs) -> ProviderPolicies {
    let model = args
        .model
        .strip_prefix(PROVIDER_PREFIX)
        .unwrap_or(args.model.as_str());
    let (prefix_region, base_model) = split_region(model);
    let region = prefix_region.unwrap_or(DEFAULT_REGION);
    let api_base = args
        .api_base
        .as_deref()
        .filter(|base| !base.is_empty())
        .map_or_else(
            || format!("https://bedrock-mantle.{region}.api.aws/v1"),
            str::to_owned,
        );
    ProviderPolicies {
        models: ModelSets::default(),
        mantle: Some(MantleEndpoint {
            api_base,
            api_key: args.api_key.clone(),
            base_model: base_model.to_owned(),
        }),
    }
}

// split_mantle_region_prefix: the head counts only when it names a Bedrock region
fn split_region(model: &str) -> (Option<&str>, &str) {
    let Some((head, tail)) = model.split_once('/') else {
        return (None, model);
    };
    let regions = AmazonBedrockGlobalConfig.get_all_regions();
    if regions.iter().any(|region| *region == head) {
        (Some(head), tail)
    } else {
        (None, model)
    }
}
