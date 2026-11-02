use std::sync::Arc;

use super::error::ProvidersError;
use super::types::ProviderCreateInfo;

const MAX_FRAGMENT_BYTES: usize = 512 * 1024;

const FRAGMENTS: [&[u8]; 10] = [
    include_bytes!("../../../assets/provider_create_fields_aiml_textract.json"),
    include_bytes!("../../../assets/provider_create_fields_mantle_azure.json"),
    include_bytes!("../../../assets/provider_create_fields_speech_cortecs.json"),
    include_bytes!("../../../assets/provider_create_fields_custom_empower.json"),
    include_bytes!("../../../assets/provider_create_fields_fal_huggingface.json"),
    include_bytes!("../../../assets/provider_create_fields_humanloop_mistral.json"),
    include_bytes!("../../../assets/provider_create_fields_minimax_openai.json"),
    include_bytes!("../../../assets/provider_create_fields_openai_like_predibase.json"),
    include_bytes!("../../../assets/provider_create_fields_prism_topaz.json"),
    include_bytes!("../../../assets/provider_create_fields_triton_watsonx.json"),
];

const FINAL_FOUR: &[u8] =
    include_bytes!("../../../data/provider_create_fields_watsonx_text_cursor.json");

fn decode_fragment(bytes: &[u8]) -> Result<Vec<ProviderCreateInfo>, ProvidersError> {
    if bytes.len() > MAX_FRAGMENT_BYTES {
        return Err(ProvidersError::FragmentTooLarge {
            size: bytes.len(),
            limit: MAX_FRAGMENT_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|error| ProvidersError::FragmentJson(Arc::new(error)))
}

pub fn provider_catalog() -> Result<Vec<ProviderCreateInfo>, ProvidersError> {
    let mut catalog = Vec::new();
    for fragment in FRAGMENTS {
        catalog.extend(decode_fragment(fragment)?);
    }
    catalog.extend(decode_fragment(FINAL_FOUR)?);
    Ok(catalog)
}
