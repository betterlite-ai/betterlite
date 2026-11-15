serde_json::json!({
    "custom_code": { "default_value": null, "description": "custom_code", "required": true, "type": "string" },
    "timeout": { "default_value": 30.0_f64, "description": "Wall-clock limit in seconds for one run of apply_guardrail, module-level code included. A run that exceeds it fails the request instead of stalling the proxy.", "required": false, "type": "number" },
    "ui_friendly_name": null,
})
