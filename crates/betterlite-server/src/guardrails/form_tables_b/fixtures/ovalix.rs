serde_json::json!({
    "application_id": { "description": "Application ID for the Ovalix Tracker service.", "required": false, "type": "string" },
    "post_checkpoint_id": { "description": "Post-checkpoint ID for the Ovalix Tracker service.", "required": false, "type": "string" },
    "pre_checkpoint_id": { "description": "Pre-checkpoint ID for the Ovalix Tracker service.", "required": false, "type": "string" },
    "tracker_api_base": { "description": "Base URL for the Ovalix Tracker service.", "required": false, "type": "string" },
    "tracker_api_key": { "description": "API key for the Ovalix Tracker service.", "required": false, "type": "string" },
    "ui_friendly_name": "Ovalix Guardrail",
})
