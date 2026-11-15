use serde_json::json;

use crate::guardrails::provider_forms::spec::{FormExtra, FormField, FormModel, FormShape};

#[rustfmt::skip]
pub(super) fn agent_365_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Microsoft Agent 365"), fields: vec![
        FormField { name: "client_id", description: Some("Client id of the gateway's Entra app registration (a confidential client). Falls back to the AGENT365_CLIENT_ID environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "client_secret", description: Some("Client secret of the gateway's Entra app registration, used to perform the On-Behalf-Of exchange. Falls back to the AGENT365_CLIENT_SECRET environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "tenant_id", description: Some("Entra tenant id used for the On-Behalf-Of token exchange. Falls back to the AGENT365_TENANT_ID environment variable."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("Behavior when Agent 365 or Entra is unreachable, times out, returns 5xx, skips the evaluation, or rejects the gateway's own client credentials. 'fail_closed' (default) blocks the tool call with HTTP 503. 'fail_open' allows it, logs an error and records it as Unscanned in the logs and OpenTelemetry. Policy blocks, 4xx rejections, throttling and a rejected caller token always block."), required: false, shape: FormShape::Choice(vec![json!("fail_closed"), json!("fail_open")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn aim_model() -> FormModel {
    FormModel { ui_friendly_name: Some("AIM Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("The API base for the Aim guardrail. Default is https://api.aim.security. Also checks if the `AIM_API_BASE` environment variable is set."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the Aim guardrail. If not provided, the `AIM_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "inspect_embeddings", description: Some("Send /embeddings `input` to Aim as user messages. Off by default because embedding input is documents being indexed, not a conversation."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: Some("bool".to_owned()), type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
pub(super) fn akto_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Akto"), fields: vec![
        FormField { name: "akto_account_id", description: Some("Akto account ID for multi-tenant deployments. Env: AKTO_ACCOUNT_ID. Default: '1000000'."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "akto_api_key", description: Some("API key for Akto. Env: AKTO_API_KEY."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "akto_base_url", description: Some("Akto Guardrail API Base URL. Env: AKTO_GUARDRAIL_API_BASE."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "akto_vxlan_id", description: Some("Akto VXLAN ID. Env: AKTO_VXLAN_ID. Default: '0'."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "guardrail_timeout", description: Some("HTTP timeout in seconds. Default: 5."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
        FormField { name: "unreachable_fallback", description: Some("What to do when Akto is unreachable. 'fail_open' = allow, 'fail_closed' = block."), required: false, shape: FormShape::Choice(vec![json!("fail_closed"), json!("fail_open")]), default: Some(json!("fail_closed")), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn cato_networks_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Cato Networks Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("The API base for the Cato Networks guardrail. Default is https://api.aisec.catonetworks.com. Also checks if the `CATO_API_BASE` environment variable is set."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The API key for the Cato Networks guardrail. If not provided, the `CATO_API_KEY` environment variable is checked."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "inspect_embeddings", description: Some("Send /embeddings `input` to Cato Networks as user messages. Off by default because embedding input is documents being indexed, not a conversation."), required: false, shape: FormShape::Boolean, default: Some(json!(false)), extra: FormExtra { ui_type: Some("bool".to_owned()), type_override: None, options: None, min: None, max: None, step: None, default_value: None } },
    ] }
}

#[rustfmt::skip]
pub(super) fn cisco_ai_defense_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Cisco AI Defense"), fields: vec![
        FormField { name: "api_base", description: Some("Regional base URL for the Cisco AI Defense Inspection API. Defaults to https://us.api.inspect.aidefense.security.cisco.com. Supported regions: us (us-west-2), ap (ap-ne-1), eu (eu-central-1). The environment variable `CISCO_AI_DEFENSE_API_BASE` is consulted as a fallback. The endpoint path is derived from inspection_type (/api/v1/inspect/chat for 'chat', /api/v1/inspect/mcp for 'mcp')."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("API key for the Cisco AI Defense inspection endpoint. If not provided, the `CISCO_AI_DEFENSE_API_KEY` environment variable is used. Sent in the `X-Cisco-AI-Defense-API-Key` header. Both the chat and MCP endpoints use this key."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(cisco_ai_defense_params())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn cisco_ai_defense_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "enabled_rules", description: Some("Explicit list of Cisco AI Defense rules to evaluate. If omitted, the policies configured for the API key in the Cisco AI Defense UI are used."), required: false, shape: FormShape::List { options: Vec::new() }, default: None, extra: FormExtra::NONE },
        FormField { name: "fallback_on_error", description: Some("Behaviour when the Cisco AI Defense API is unavailable: 'allow' proceeds without scanning (high availability), 'block' rejects the request (maximum security)."), required: false, shape: FormShape::Choice(vec![json!("allow"), json!("block")]), default: Some(json!("block")), extra: FormExtra::NONE },
        FormField { name: "inspect_path", description: Some("Override for the inspection endpoint path. Defaults to /api/v1/inspect/chat when inspection_type='chat' and /api/v1/inspect/mcp when inspection_type='mcp'."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "inspection_type", description: Some("Which Cisco AI Defense inspection surface to use. 'chat' scans LLM model conversations via /api/v1/inspect/chat. 'mcp' scans MCP tool calls via /api/v1/inspect/mcp. Each guardrail instance targets exactly one surface; configure two guardrails to scan both chat and MCP traffic."), required: false, shape: FormShape::Choice(vec![json!("chat"), json!("mcp")]), default: Some(json!("chat")), extra: FormExtra::NONE },
        FormField { name: "integration_profile_id", description: Some("Integration profile id to apply (advanced)."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "integration_profile_version", description: Some("Integration profile version to apply (advanced)."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "integration_tenant_id", description: Some("Integration tenant id to apply (advanced)."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "integration_type", description: Some("Integration type to apply (advanced)."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "on_flagged_action", description: Some("Action to take when Cisco AI Defense flags content. 'block' raises an HTTPException; 'monitor' logs the detection and lets the request continue."), required: false, shape: FormShape::Text, default: Some(json!("block")), extra: FormExtra::NONE },
        FormField { name: "timeout", description: Some("Timeout (seconds) for Cisco AI Defense API calls (1-60)."), required: false, shape: FormShape::Number, default: Some(json!(10.0_f64)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn ibm_guardrails_model() -> FormModel {
    FormModel { ui_friendly_name: Some("IBM Guardrails Detector"), fields: vec![
        FormField { name: "auth_token", description: Some("Authorization bearer token for IBM Guardrails API. Reads from IBM_GUARDRAILS_AUTH_TOKEN env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "base_url", description: Some("Base URL for the IBM Guardrails server"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "detector_id", description: Some("Name of the detector inside the server (e.g., 'jailbreak-detector')"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "is_detector_server", description: Some("Boolean flag to determine if calling a detector server (True) or the FMS Orchestrator (False). Defaults to True."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the IBM Detector guardrail"), required: false, shape: FormShape::Nested(Box::new(ibm_guardrails_params())), default: None, extra: FormExtra::NONE },
        FormField { name: "verify_ssl", description: Some("Whether to verify SSL certificates. Defaults to True."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn ibm_guardrails_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "block_on_detection", description: Some("Whether to block requests when detections are found. Defaults to True."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "detector_params", description: Some("Dictionary of arguments to pass to the detector."), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Text) }, default: Some(json!(null)), extra: FormExtra::NONE },
        FormField { name: "extra_headers", description: Some("Dictionary of extra headers to pass to the detector."), required: false, shape: FormShape::Mapping { keys: Vec::new(), value: Box::new(FormShape::Text) }, default: Some(json!(null)), extra: FormExtra::NONE },
        FormField { name: "score_threshold", description: Some("Minimum score threshold to consider a detection as a violation (0.0 to 1.0). If set, detections below this threshold will be ignored."), required: false, shape: FormShape::Number, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn model_armor_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Google Cloud Model Armor"), fields: vec![
        FormField { name: "api_endpoint", description: Some("Optional custom API endpoint for Model Armor"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "credentials", description: Some("Path to Google Cloud credentials JSON file or JSON string"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "fail_on_error", description: Some("Whether to fail the request if Model Armor encounters an error"), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "location", description: Some("Google Cloud location/region (e.g., us-central1)"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "project_id", description: Some("Google Cloud project ID"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "sanitize_error_detail", description: Some("Omit the raw Model Armor response from caller-facing errors and logs by default. Set False to restore verbose output."), required: false, shape: FormShape::Boolean, default: Some(json!(true)), extra: FormExtra::NONE },
        FormField { name: "template_id", description: Some("The ID of your Model Armor template"), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn pangea_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Pangea Guardrail"), fields: vec![
        FormField { name: "api_base", description: Some("The Pangea API base URL. Defaults to https://ai-guard.aws.us.pangea.cloud. Also checks if the PANGEA_API_BASE env var is set."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "api_key", description: Some("The Pangea API key. Reads from PANGEA_API_KEY env var if None."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: FormShape::Nested(Box::new(pangea_params())), default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn pangea_params() -> FormModel {
    FormModel { ui_friendly_name: None, fields: vec![
        FormField { name: "pangea_input_recipe", description: Some("The Pangea input recipe for the Pangea guardrail. Used for pre-call hook."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "pangea_output_recipe", description: Some("The Pangea output recipe for the Pangea guardrail. Used for post-call hook."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}

#[rustfmt::skip]
pub(super) fn singulr_model() -> FormModel {
    FormModel { ui_friendly_name: Some("Singulr"), fields: vec![
        FormField { name: "block_on_error", description: Some("Whether to block requests when the Singulr Guardrails API is unavailable or returns an error. If enabled, requests fail closed. If disabled, requests continue without guardrail enforcement (fail open)."), required: false, shape: FormShape::Boolean, default: None, extra: FormExtra::NONE },
        FormField { name: "singulr_api_base", description: Some("The Singulr API base URL. Get base URL from Singulr Platform."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "singulr_api_key", description: Some("The Singulr API key. Generate API key from Singulr Platform."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "singulr_application_id", description: Some("The Singulr application ID. Get application ID from Singulr Platform."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
        FormField { name: "singulr_guardrail_id", description: Some("The Singulr Guardrail ID. Get guardrail ID from Singulr Platform."), required: false, shape: FormShape::Text, default: None, extra: FormExtra::NONE },
    ] }
}
