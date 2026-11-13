use std::collections::BTreeSet;

use serde::Serialize;

use super::EndpointRegistry;

// public_endpoints.py's _ENDPOINT_METADATA, which labels and paths each route in the projection.
const ENDPOINT_METADATA: [(&str, &str, &str); 35] = [
    ("chat_completions", "Chat Completions", "/chat/completions"),
    ("messages", "Messages", "/messages"),
    ("responses", "Responses", "/responses"),
    ("embeddings", "Embeddings", "/embeddings"),
    (
        "image_generations",
        "Image Generations",
        "/images/generations",
    ),
    (
        "audio_transcriptions",
        "Audio Transcriptions",
        "/audio/transcriptions",
    ),
    ("audio_speech", "Audio Speech", "/audio/speech"),
    ("moderations", "Moderations", "/moderations"),
    ("batches", "Batches", "/batches"),
    ("rerank", "Rerank", "/rerank"),
    ("ocr", "OCR", "/ocr"),
    ("search", "Search", "/search"),
    ("skills", "Skills", "/skills"),
    ("interactions", "Interactions", "/interactions"),
    ("a2a", "A2A (Agent Gateway)", "/a2a/{agent}/message/send"),
    ("container", "Containers", "/containers"),
    (
        "container_files",
        "Container Files",
        "/containers/{id}/files",
    ),
    ("compact", "Compact", "/responses/compact"),
    ("files", "Files", "/files"),
    ("image_edits", "Image Edits", "/images/edits"),
    (
        "vector_stores_create",
        "Vector Stores (Create)",
        "/vector_stores",
    ),
    (
        "vector_stores_search",
        "Vector Stores (Search)",
        "/vector_stores/{id}/search",
    ),
    (
        "vector_store_files",
        "Vector Store Files",
        "/vector_stores/{id}/files",
    ),
    (
        "video_generations",
        "Video Generations",
        "/videos/generations",
    ),
    ("assistants", "Assistants", "/assistants"),
    ("fine_tuning", "Fine Tuning", "/fine_tuning/jobs"),
    ("text_completion", "Text Completion", "/completions"),
    ("realtime", "Realtime", "/realtime"),
    ("count_tokens", "Count Tokens", "/utils/token_counter"),
    ("image_variations", "Image Variations", "/images/variations"),
    ("generateContent", "Generate Content", "/generateContent"),
    ("bedrock_invoke", "Bedrock Invoke", "/bedrock/invoke"),
    ("bedrock_converse", "Bedrock Converse", "/bedrock/converse"),
    ("rag_ingest", "RAG Ingest", "/rag/ingest"),
    ("rag_query", "RAG Query", "/rag/query"),
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EndpointProjection {
    pub key: String,
    pub label: String,
    pub endpoint: String,
    pub providers: Vec<ProviderProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderProjection {
    pub slug: String,
    pub display_name: String,
}

#[must_use]
pub fn project(registry: &EndpointRegistry) -> Vec<EndpointProjection> {
    let mut keys: Vec<&str> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for capabilities in registry.providers.values() {
        for key in capabilities.endpoints.keys() {
            if seen.insert(key.as_str()) {
                keys.push(key.as_str());
            }
        }
    }
    keys.into_iter()
        .map(|key| project_endpoint(registry, key))
        .collect()
}

fn project_endpoint(registry: &EndpointRegistry, key: &str) -> EndpointProjection {
    let (label, endpoint) = metadata(key);
    let providers = registry
        .providers
        .iter()
        .filter(|(_, capabilities)| capabilities.endpoints.get(key).copied().unwrap_or(false))
        .map(|(slug, capabilities)| ProviderProjection {
            slug: slug.clone(),
            display_name: clean_display_name(capabilities.display_name.as_deref().unwrap_or(slug)),
        })
        .collect();
    EndpointProjection {
        key: key.to_owned(),
        label,
        endpoint,
        providers,
    }
}

fn metadata(key: &str) -> (String, String) {
    match ENDPOINT_METADATA.iter().find(|(name, _, _)| *name == key) {
        Some((_, label, endpoint)) => ((*label).to_owned(), (*endpoint).to_owned()),
        None => (
            title_case(&key.replace('_', " ")),
            format!("/{}", key.replace('_', "/")),
        ),
    }
}

#[must_use]
pub fn clean_display_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some(without_close) = trimmed.strip_suffix(')') else {
        return trimmed.to_owned();
    };
    let Some((prefix, inner)) = without_close.rsplit_once('(') else {
        return trimmed.to_owned();
    };
    let Some(slug) = inner
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
    else {
        return trimmed.to_owned();
    };
    if slug.is_empty() || slug.contains('`') {
        return trimmed.to_owned();
    }
    prefix.trim().to_owned()
}

fn title_case(text: &str) -> String {
    let mut title = String::with_capacity(text.len());
    let mut word_start = true;
    for character in text.chars() {
        if character.is_alphabetic() {
            if word_start {
                title.extend(character.to_uppercase());
            } else {
                title.extend(character.to_lowercase());
            }
            word_start = false;
        } else {
            title.push(character);
            word_start = true;
        }
    }
    title
}
