serde_json::json!({
    "auth_token": { "description": "Authorization bearer token for IBM Guardrails API. Reads from IBM_GUARDRAILS_AUTH_TOKEN env var if None.", "required": false, "type": "string" },
    "base_url": { "description": "Base URL for the IBM Guardrails server", "required": false, "type": "string" },
    "detector_id": { "description": "Name of the detector inside the server (e.g., 'jailbreak-detector')", "required": false, "type": "string" },
    "is_detector_server": { "default_value": true, "description": "Boolean flag to determine if calling a detector server (True) or the FMS Orchestrator (False). Defaults to True.", "required": false, "type": "boolean" },
    "optional_params": { "description": "Optional parameters for the IBM Detector guardrail", "fields": { "block_on_detection": { "default_value": true, "description": "Whether to block requests when detections are found. Defaults to True.", "required": false, "type": "boolean" }, "detector_params": { "default_value": null, "description": "Dictionary of arguments to pass to the detector.", "dict_value_type": "string", "required": false, "type": "dict" }, "extra_headers": { "default_value": null, "description": "Dictionary of extra headers to pass to the detector.", "dict_value_type": "string", "required": false, "type": "dict" }, "score_threshold": { "description": "Minimum score threshold to consider a detection as a violation (0.0 to 1.0). If set, detections below this threshold will be ignored.", "required": false, "type": "number" } }, "required": false, "type": "nested" },
    "ui_friendly_name": "IBM Guardrails Detector",
    "verify_ssl": { "default_value": true, "description": "Whether to verify SSL certificates. Defaults to True.", "required": false, "type": "boolean" },
})
