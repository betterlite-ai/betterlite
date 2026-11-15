serde_json::json!({
    "default_action": { "default_value": "deny", "description": "Fallback decision when no rule matches", "options": ["allow", "deny"], "required": false, "type": "select" },
    "on_disallowed_action": { "default_value": "block", "description": "Choose whether disallowed tools block the request or get rewritten out of the payload", "options": ["block", "rewrite"], "required": false, "type": "select" },
    "rules": { "description": "Ordered allow/deny rules. Patterns use regex for tool names/types and optional regex constraints on tool arguments.", "required": false, "type": "array" },
    "ui_friendly_name": "LiteLLM Tool Permission Guardrail",
})
