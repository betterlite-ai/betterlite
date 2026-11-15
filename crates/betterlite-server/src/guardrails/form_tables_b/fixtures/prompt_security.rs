serde_json::json!({
    "api_base": { "description": "The API base for the Prompt Security guardrail. If not provided, the `PROMPT_SECURITY_API_BASE` environment variable is used.", "required": false, "type": "string" },
    "api_key": { "description": "The API key for the Prompt Security guardrail. If not provided, the `PROMPT_SECURITY_API_KEY` environment variable is used.", "required": false, "type": "string" },
    "block_on_file_modify": { "default_value": true, "description": "Whether a file sanitization `modify` verdict blocks the request instead of replacing the file content.", "required": false, "type": "boolean" },
    "file_sanitization_fail_open": { "default_value": true, "description": "Whether file sanitization timeouts allow the original file through instead of blocking the request.", "required": false, "type": "boolean" },
    "streaming_transform_mode": { "description": "How post_call `modify` verdicts reach a streaming client. `block_only` (default) streams the raw upstream chunks and only a `block` verdict ends the stream, so `modified_text` is dropped. `incremental_diff` buffers the whole response and sends the redacted text once the final verdict is in, so the first token arrives with the last, while a `block` verdict still ends the stream early. OpenAI chat completions streaming only.", "options": ["block_only", "incremental_diff"], "required": false, "type": "select" },
    "ui_friendly_name": "Prompt Security",
})
