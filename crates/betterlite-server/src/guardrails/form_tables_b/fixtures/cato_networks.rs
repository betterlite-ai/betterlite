serde_json::json!({
    "api_base": { "description": "The API base for the Cato Networks guardrail. Default is https://api.aisec.catonetworks.com. Also checks if the `CATO_API_BASE` environment variable is set.", "required": false, "type": "string" },
    "api_key": { "description": "The API key for the Cato Networks guardrail. If not provided, the `CATO_API_KEY` environment variable is checked.", "required": false, "type": "string" },
    "inspect_embeddings": { "default_value": false, "description": "Send /embeddings `input` to Cato Networks as user messages. Off by default because embedding input is documents being indexed, not a conversation.", "required": false, "type": "bool" },
    "ui_friendly_name": "Cato Networks Guardrail",
})
