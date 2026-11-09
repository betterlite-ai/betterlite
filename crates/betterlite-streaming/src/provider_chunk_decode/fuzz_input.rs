use std::str::from_utf8;

use uuid::Uuid;

use crate::provider_chunk_decode::chunk::{
    DecodeState, DecodedChunk, ProviderChunk, ProviderKind, RequestOptions,
};
use crate::provider_chunk_decode::dispatch::decode_provider_chunk;
use crate::provider_chunk_decode::error::ProviderChunkError;
use crate::stream_chunk_shaping::extras::generic_chunk_has_known_fields;
use crate::stream_chunk_shaping::records::{GenericChunk, JsonValue};

// The provider byte's low four bits pick the reader, and bit six marks a custom provider.
const CUSTOM_FLAG: u8 = 0x40;

const PROVIDERS: [Option<&str>; 16] = [
    None,
    Some("openai"),
    Some("azure"),
    Some("azure_ai"),
    Some("replicate"),
    Some("predibase"),
    Some("vllm"),
    Some("aleph_alpha"),
    Some("nlp_cloud"),
    Some("vertex_ai"),
    Some("petals"),
    Some("triton"),
    Some("text-completion-openai"),
    Some("text-completion-codestral"),
    Some("azure_text"),
    Some("cached_response"),
];

// The fuzz target and the corpus replay read the same bytes through this one function.
#[must_use]
pub fn decode_fuzzed_stream_chunk(
    bytes: &[u8],
) -> Option<Result<DecodedChunk, ProviderChunkError>> {
    let (&selector, rest) = bytes.split_first()?;
    let provider = PROVIDERS
        .get(usize::from(selector & 0x0f))
        .copied()
        .flatten();
    let kind = if selector & CUSTOM_FLAG == 0 {
        ProviderKind::Builtin
    } else {
        ProviderKind::Custom
    };
    let mut state = DecodeState {
        model: "gpt-4".to_owned(),
        kind,
        received_finish_reason: None,
        intermittent_finish_reason: None,
        sent_first_chunk: false,
        complete_response: String::new(),
        completion_stream: from_utf8(rest).ok().map(str::to_owned),
        options: RequestOptions {
            send_stream_usage: true,
            is_function_call: true,
        },
        repeated_chunk_limit: 100,
        next_id: Uuid::new_v4,
    };
    Some(decode_provider_chunk(provider, &chunk_of(rest), &mut state))
}

fn chunk_of(bytes: &[u8]) -> ProviderChunk {
    let Ok(text) = from_utf8(bytes) else {
        return ProviderChunk::Bytes(bytes.to_vec());
    };
    let Ok(value) = serde_json::from_str::<JsonValue>(text) else {
        return ProviderChunk::Text(text.to_owned());
    };
    if let JsonValue::Object(fields) = &value
        && generic_chunk_has_known_fields(fields)
        && let Ok(plain) = serde_json::to_value(&value)
        && let Ok(generic) = serde_json::from_value::<GenericChunk>(plain)
    {
        return ProviderChunk::Generic(Box::new(generic));
    }
    ProviderChunk::Json(Box::new(value))
}
