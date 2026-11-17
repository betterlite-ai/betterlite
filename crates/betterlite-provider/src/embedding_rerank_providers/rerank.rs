use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::{cohere_rerank_settings, jina_infinity_rerank};
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_openai::cohere_rerank::{self, RerankVersion};
use betterlite_openai::host_rerank::{self, HostProvider};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::voyage_rerank;
use uuid::Uuid;

use crate::api::rerank_contracts::Params;
use crate::embedding_rerank_providers::error::FamilyError;
use crate::embedding_rerank_providers::request::{http_headers, url};
use crate::embedding_rerank_providers::{Call, Decode, EmbeddingRerankPrepared};

mod config;
mod voyage;

// a rerank answer is small next to an embedding batch, and these are the limits the host leaves use
const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0080_0000,
    max_event_bytes: 0x0080_0000,
    max_items: 200_000,
};

// B1014's dispatch builds this for the family it selected and calls the preparation contract on it
pub struct FamilyRerankConfig<'settings> {
    pub config: RerankConfig,
    pub cohere: &'settings cohere_rerank_settings::Settings,
    pub host: &'settings jina_infinity_rerank::Settings,
    pub environment: &'settings (dyn EnvironmentLookup + Send + Sync),
}

impl FamilyRerankConfig<'_> {
    // the contract's get_complete_url has no error path, so preparation calls this one to keep the failure
    pub fn url(&self, api_base: Option<&str>) -> Result<String, FamilyError> {
        match self.config {
            RerankConfig::Cohere => Ok(cohere_rerank::complete_url(RerankVersion::V1, api_base)),
            RerankConfig::CohereV2 => Ok(cohere_rerank::complete_url(RerankVersion::V2, api_base)),
            RerankConfig::Infinity => Ok(host_rerank::infinity_complete_url(api_base)?),
            RerankConfig::JinaAi => Ok(host_rerank::jina_complete_url(api_base)?),
            RerankConfig::Voyage => Ok(voyage_rerank::endpoint(api_base)),
        }
    }
}

// the prepared call the dispatch sends, built from the contract's URL, headers and transformed body
pub fn prepare(
    config: RerankConfig,
    endpoint: &str,
    headers: &Headers,
    body: &Params,
    timeout: Duration,
) -> Result<EmbeddingRerankPrepared, FamilyError> {
    let family = match config {
        RerankConfig::Cohere | RerankConfig::CohereV2 => "cohere",
        RerankConfig::Infinity => "infinity",
        RerankConfig::JinaAi => "jina",
        RerankConfig::Voyage => "voyage",
    };
    Ok(EmbeddingRerankPrepared(Call {
        url: url(family, endpoint)?,
        headers: http_headers(headers)?,
        body: serde_json::to_vec(body)?,
        timeout,
        decode: Decode::Rerank(config),
    }))
}

// a rerank answer from any of the four families, in the shared rerank response records
pub fn decode(
    config: RerankConfig,
    status: u16,
    body: &[u8],
) -> Result<RerankResponse, FamilyError> {
    Ok(match config {
        RerankConfig::Cohere | RerankConfig::CohereV2 => {
            cohere_rerank::transform_rerank_response(status, body, LIMITS)?
        }
        RerankConfig::Infinity => host_rerank::decode_host_rerank(
            HostProvider::Infinity,
            status,
            body,
            Uuid::new_v4,
            |_| {},
        )?,
        RerankConfig::JinaAi => {
            host_rerank::decode_host_rerank(HostProvider::Jina, status, body, Uuid::new_v4, |_| {})?
        }
        RerankConfig::Voyage => voyage::voyage_response(status, body)?,
    })
}
