use betterlite_streaming::stream_base_assembly::StreamedChunks;
use betterlite_streaming::stream_base_assembly::records::Chunk;
use betterlite_streaming::stream_base_assembly::response::{
    BaseResponse, apply_hidden_params, build_base_response,
};
use betterlite_streaming::stream_chunk_shaping::records::Usage;
use serde_json::{Value, json};

use super::ResponseSpec;

pub(super) fn sort_view(chunks: &[Chunk]) -> Value {
    with_chunks(chunks, |streamed| {
        json!({
            "chunks": streamed.chunks.iter().map(chunk_view).collect::<Vec<_>>(),
            "first_chunk": chunk_view(&streamed.first_chunk),
        })
    })
}

pub(super) fn response_json(chunks: &[Chunk]) -> Value {
    with_chunks(chunks, |streamed| match build_base_response(streamed) {
        Ok(response) => response_view(&response),
        Err(_error) => json!({"ok": false, "error_type": "missing_first_field"}),
    })
}

fn with_chunks<F>(chunks: &[Chunk], view: F) -> Value
where
    F: FnOnce(&StreamedChunks) -> Value,
{
    match StreamedChunks::new(chunks.to_vec()) {
        Ok(streamed) => view(&streamed),
        Err(_error) => json!({"ok": false, "error_type": "empty_chunks"}),
    }
}

pub(super) fn update_json(response: &ResponseSpec, chunk: Option<&Chunk>) -> Value {
    let mut base = fixture_response(response);
    apply_hidden_params(&mut base, chunk);
    json!({"hidden_params": &base.hidden_params})
}

pub(super) fn response_view(response: &BaseResponse) -> Value {
    let choices: Vec<Value> = response
        .choices
        .iter()
        .map(|choice| {
            json!({
                "index": choice.index,
                "message": {"role": &choice.message.role, "content": &choice.message.content},
                "finish_reason": &choice.finish_reason,
                "provider_specific_fields": &choice.provider_specific_fields,
            })
        })
        .collect();
    json!({
        "id": &response.id,
        "object": &response.object,
        "created": response.created,
        "model": &response.model,
        "system_fingerprint": &response.system_fingerprint,
        "service_tier": &response.service_tier,
        "choices": choices,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
        },
        "hidden_params": &response.hidden_params,
    })
}

pub(super) fn fixture_response(spec: &ResponseSpec) -> BaseResponse {
    BaseResponse {
        id: "fixture".to_owned(),
        object: "chat.completion".to_owned(),
        created: 1,
        model: spec.model.clone(),
        system_fingerprint: None,
        service_tier: None,
        choices: Vec::new(),
        usage: Usage::default(),
        hidden_params: spec.hidden_params.clone(),
    }
}

fn chunk_view(chunk: &Chunk) -> Value {
    json!({
        "id": &chunk.id,
        "created": chunk.created,
        "model": &chunk.model,
        "hidden_params": hidden_view(chunk),
    })
}

fn hidden_view(chunk: &Chunk) -> Value {
    let Some(hidden) = chunk.hidden_object() else {
        return json!({});
    };
    json!(hidden)
}
