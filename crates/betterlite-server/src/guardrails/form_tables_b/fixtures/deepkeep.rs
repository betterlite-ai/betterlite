serde_json::json!({
    "api_base": { "description": "The API base URL for the DeepKeep AI Firewall. If not provided, the `DEEPKEEP_API_BASE` environment variable is checked.", "required": false, "type": "string" },
    "api_key": { "description": "The API key for the DeepKeep AI Firewall. If not provided, the `DEEPKEEP_API_KEY` environment variable is checked.", "required": false, "type": "string" },
    "deepkeep_firewall_id": { "description": "The DeepKeep Firewall ID to use for guardrail evaluation. If not provided, the `DEEPKEEP_FIREWALL_ID` environment variable is checked.", "required": false, "type": "string" },
    "optional_params": { "description": "Optional parameters for the guardrail", "fields": { "unreachable_fallback": { "default_value": "fail_closed", "description": "Behavior when the DeepKeep API is unreachable. 'fail_closed' raises an error (default). 'fail_open' logs a critical error and allows the request to proceed.", "required": false, "type": "string" } }, "required": false, "type": "nested" },
    "ui_friendly_name": "DeepKeep AI Firewall",
})
