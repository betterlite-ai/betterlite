serde_json::json!({
    "api_base": { "description": "The API base for the Aporia guardrail. If not provided, the `APORIA_API_BASE` environment variable is checked.", "required": false, "type": "string" },
    "api_key": { "description": "The API key for the Aporia guardrail. If not provided, the `APORIA_API_KEY` environment variable is checked.", "required": false, "type": "string" },
    "ui_friendly_name": "Aporia AI",
})
