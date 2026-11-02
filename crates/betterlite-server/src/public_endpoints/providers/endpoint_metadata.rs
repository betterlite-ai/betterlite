const ENDPOINT_METADATA: &[(&str, &str, &str)] = &[
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

fn title_case(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalize_next = true;
    for letter in text.chars() {
        if letter.is_alphabetic() {
            if capitalize_next {
                result.extend(letter.to_uppercase());
            } else {
                result.extend(letter.to_lowercase());
            }
            capitalize_next = false;
        } else {
            result.push(letter);
            capitalize_next = true;
        }
    }
    result
}

pub(super) fn label_and_path(key: &str) -> (String, String) {
    if let Some((_, label, path)) = ENDPOINT_METADATA.iter().find(|entry| entry.0 == key) {
        return ((*label).to_owned(), (*path).to_owned());
    }
    let label = title_case(&key.replace('_', " "));
    let path = format!("/{}", key.replace('_', "/"));
    (label, path)
}
