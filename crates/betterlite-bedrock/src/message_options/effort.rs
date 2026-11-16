use betterlite_anthropic::anthropic_capability_catalog::capability::supports_model_capability;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::bedrock_model_capabilities::effort::normalize_opus_effort;
use crate::model_routes::{ParamMap, ParamValue};

use super::{ADAPTIVE_THINKING, BEDROCK_PROVIDER, MessageOptionsError, OptionsSources};

pub async fn clamp_adaptive_effort<Source, Transport, Lookup>(
    model: &str,
    params: &mut ParamMap,
    sources: &mut OptionsSources<'_, '_, Source, Transport>,
    lookup: &Lookup,
    packaged: &ModelCatalog,
) -> Result<(), MessageOptionsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
    Lookup: ModelInfoLookup,
{
    if !supports_model_capability(
        model,
        ADAPTIVE_THINKING,
        BEDROCK_PROVIDER,
        sources.environment,
        sources.catalogs,
    )
    .await
    {
        return Ok(());
    }
    let Some(ParamValue::Text(effort)) = params.get("reasoning_effort").cloned() else {
        return Ok(());
    };
    let mut output_config = IndexMap::new();
    output_config.insert("effort".to_owned(), ParamValue::Text(effort));
    let mut clamped = ParamValue::Mapping(output_config);
    normalize_opus_effort(model, &mut clamped, lookup, packaged).await?;
    let ParamValue::Mapping(fields) = &clamped else {
        return Ok(());
    };
    if let Some(ceiling) = fields.get("effort") {
        params.insert("reasoning_effort".to_owned(), ceiling.clone());
    }
    Ok(())
}
