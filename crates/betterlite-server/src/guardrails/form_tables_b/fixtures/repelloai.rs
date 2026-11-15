serde_json::json!({
    "api_base": { "description": "Base URL for the RepelloAI Argus API. Defaults to https://argusapi.repello.ai/sdk/v1", "required": false, "type": "string" },
    "api_key": { "description": "API key for the RepelloAI Argus service. Falls back to ARGUS_API_KEY or REPELLOAI_API_KEY.", "required": false, "type": "string" },
    "asset_id": { "description": "Repello asset ID whose dashboard policies are enforced. Required; the guardrail raises at init if it is missing.", "required": false, "type": "string" },
    "optional_params": { "description": "Optional parameters for the guardrail", "required": false, "type": "string" },
    "ui_friendly_name": "RepelloAI Argus",
    "unreachable_fallback": { "default_value": "fail_closed", "description": "What to do when the RepelloAI Argus API is unreachable. 'fail_closed' = block (default), 'fail_open' = allow.", "options": ["fail_closed", "fail_open"], "required": false, "type": "select" },
})
