serde_json::json!({
    "client_id": { "description": "Client id of the gateway's Entra app registration (a confidential client). Falls back to the AGENT365_CLIENT_ID environment variable.", "required": false, "type": "string" },
    "client_secret": { "description": "Client secret of the gateway's Entra app registration, used to perform the On-Behalf-Of exchange. Falls back to the AGENT365_CLIENT_SECRET environment variable.", "required": false, "type": "string" },
    "tenant_id": { "description": "Entra tenant id used for the On-Behalf-Of token exchange. Falls back to the AGENT365_TENANT_ID environment variable.", "required": false, "type": "string" },
    "ui_friendly_name": "Microsoft Agent 365",
    "unreachable_fallback": { "default_value": "fail_closed", "description": "Behavior when Agent 365 or Entra is unreachable, times out, returns 5xx, skips the evaluation, or rejects the gateway's own client credentials. 'fail_closed' (default) blocks the tool call with HTTP 503. 'fail_open' allows it, logs an error and records it as Unscanned in the logs and OpenTelemetry. Policy blocks, 4xx rejections, throttling and a rejected caller token always block.", "options": ["fail_closed", "fail_open"], "required": false, "type": "select" },
})
