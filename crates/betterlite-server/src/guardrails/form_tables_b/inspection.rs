use super::data::{SpecField, SpecModel, SpecShape, SpecValue};

#[rustfmt::skip]
pub(super) const AGENT_365: SpecModel = SpecModel { ui_friendly_name: Some("Microsoft Agent 365"), fields: &[
    SpecField { name: "client_id", description: Some("Client id of the gateway's Entra app registration (a confidential client). Falls back to the AGENT365_CLIENT_ID environment variable."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "client_secret", description: Some("Client secret of the gateway's Entra app registration, used to perform the On-Behalf-Of exchange. Falls back to the AGENT365_CLIENT_SECRET environment variable."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "tenant_id", description: Some("Entra tenant id used for the On-Behalf-Of token exchange. Falls back to the AGENT365_TENANT_ID environment variable."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "unreachable_fallback", description: Some("Behavior when Agent 365 or Entra is unreachable, times out, returns 5xx, skips the evaluation, or rejects the gateway's own client credentials. 'fail_closed' (default) blocks the tool call with HTTP 503. 'fail_open' allows it, logs an error and records it as Unscanned in the logs and OpenTelemetry. Policy blocks, 4xx rejections, throttling and a rejected caller token always block."), required: false, shape: SpecShape::Choice(&["fail_closed", "fail_open"]), default: Some(SpecValue::Text("fail_closed")), ui_type: None },
] };

#[rustfmt::skip]
pub(super) const AIM: SpecModel = SpecModel { ui_friendly_name: Some("AIM Guardrail"), fields: &[
    SpecField { name: "api_base", description: Some("The API base for the Aim guardrail. Default is https://api.aim.security. Also checks if the `AIM_API_BASE` environment variable is set."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "api_key", description: Some("The API key for the Aim guardrail. If not provided, the `AIM_API_KEY` environment variable is checked."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "inspect_embeddings", description: Some("Send /embeddings `input` to Aim as user messages. Off by default because embedding input is documents being indexed, not a conversation."), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(false)), ui_type: Some("bool") },
] };

#[rustfmt::skip]
pub(super) const AKTO: SpecModel = SpecModel { ui_friendly_name: Some("Akto"), fields: &[
    SpecField { name: "akto_account_id", description: Some("Akto account ID for multi-tenant deployments. Env: AKTO_ACCOUNT_ID. Default: '1000000'."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "akto_api_key", description: Some("API key for Akto. Env: AKTO_API_KEY."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "akto_base_url", description: Some("Akto Guardrail API Base URL. Env: AKTO_GUARDRAIL_API_BASE."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "akto_vxlan_id", description: Some("Akto VXLAN ID. Env: AKTO_VXLAN_ID. Default: '0'."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "guardrail_timeout", description: Some("HTTP timeout in seconds. Default: 5."), required: false, shape: SpecShape::Number, default: None, ui_type: None },
    SpecField { name: "unreachable_fallback", description: Some("What to do when Akto is unreachable. 'fail_open' = allow, 'fail_closed' = block."), required: false, shape: SpecShape::Choice(&["fail_closed", "fail_open"]), default: Some(SpecValue::Text("fail_closed")), ui_type: None },
] };

#[rustfmt::skip]
pub(super) const CATO_NETWORKS: SpecModel = SpecModel { ui_friendly_name: Some("Cato Networks Guardrail"), fields: &[
    SpecField { name: "api_base", description: Some("The API base for the Cato Networks guardrail. Default is https://api.aisec.catonetworks.com. Also checks if the `CATO_API_BASE` environment variable is set."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "api_key", description: Some("The API key for the Cato Networks guardrail. If not provided, the `CATO_API_KEY` environment variable is checked."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "inspect_embeddings", description: Some("Send /embeddings `input` to Cato Networks as user messages. Off by default because embedding input is documents being indexed, not a conversation."), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(false)), ui_type: Some("bool") },
] };

#[rustfmt::skip]
pub(super) const CISCO_AI_DEFENSE: SpecModel = SpecModel { ui_friendly_name: Some("Cisco AI Defense"), fields: &[
    SpecField { name: "api_base", description: Some("Regional base URL for the Cisco AI Defense Inspection API. Defaults to https://us.api.inspect.aidefense.security.cisco.com. Supported regions: us (us-west-2), ap (ap-ne-1), eu (eu-central-1). The environment variable `CISCO_AI_DEFENSE_API_BASE` is consulted as a fallback. The endpoint path is derived from inspection_type (/api/v1/inspect/chat for 'chat', /api/v1/inspect/mcp for 'mcp')."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "api_key", description: Some("API key for the Cisco AI Defense inspection endpoint. If not provided, the `CISCO_AI_DEFENSE_API_KEY` environment variable is used. Sent in the `X-Cisco-AI-Defense-API-Key` header. Both the chat and MCP endpoints use this key."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: SpecShape::Nested(CISCO_AI_DEFENSE_PARAMS), default: None, ui_type: None },
] };

#[rustfmt::skip]
pub(super) const CISCO_AI_DEFENSE_PARAMS: &[SpecField] = &[
    SpecField { name: "enabled_rules", description: Some("Explicit list of Cisco AI Defense rules to evaluate. If omitted, the policies configured for the API key in the Cisco AI Defense UI are used."), required: false, shape: SpecShape::List(&[]), default: None, ui_type: None },
    SpecField { name: "fallback_on_error", description: Some("Behaviour when the Cisco AI Defense API is unavailable: 'allow' proceeds without scanning (high availability), 'block' rejects the request (maximum security)."), required: false, shape: SpecShape::Choice(&["allow", "block"]), default: Some(SpecValue::Text("block")), ui_type: None },
    SpecField { name: "inspect_path", description: Some("Override for the inspection endpoint path. Defaults to /api/v1/inspect/chat when inspection_type='chat' and /api/v1/inspect/mcp when inspection_type='mcp'."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "inspection_type", description: Some("Which Cisco AI Defense inspection surface to use. 'chat' scans LLM model conversations via /api/v1/inspect/chat. 'mcp' scans MCP tool calls via /api/v1/inspect/mcp. Each guardrail instance targets exactly one surface; configure two guardrails to scan both chat and MCP traffic."), required: false, shape: SpecShape::Choice(&["chat", "mcp"]), default: Some(SpecValue::Text("chat")), ui_type: None },
    SpecField { name: "integration_profile_id", description: Some("Integration profile id to apply (advanced)."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "integration_profile_version", description: Some("Integration profile version to apply (advanced)."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "integration_tenant_id", description: Some("Integration tenant id to apply (advanced)."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "integration_type", description: Some("Integration type to apply (advanced)."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "on_flagged_action", description: Some("Action to take when Cisco AI Defense flags content. 'block' raises an HTTPException; 'monitor' logs the detection and lets the request continue."), required: false, shape: SpecShape::Text, default: Some(SpecValue::Text("block")), ui_type: None },
    SpecField { name: "timeout", description: Some("Timeout (seconds) for Cisco AI Defense API calls (1-60)."), required: false, shape: SpecShape::Number, default: Some(SpecValue::Float(10.0)), ui_type: None },
];

#[rustfmt::skip]
pub(super) const IBM_GUARDRAILS: SpecModel = SpecModel { ui_friendly_name: Some("IBM Guardrails Detector"), fields: &[
    SpecField { name: "auth_token", description: Some("Authorization bearer token for IBM Guardrails API. Reads from IBM_GUARDRAILS_AUTH_TOKEN env var if None."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "base_url", description: Some("Base URL for the IBM Guardrails server"), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "detector_id", description: Some("Name of the detector inside the server (e.g., 'jailbreak-detector')"), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "is_detector_server", description: Some("Boolean flag to determine if calling a detector server (True) or the FMS Orchestrator (False). Defaults to True."), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(true)), ui_type: None },
    SpecField { name: "optional_params", description: Some("Optional parameters for the IBM Detector guardrail"), required: false, shape: SpecShape::Nested(IBM_GUARDRAILS_PARAMS), default: None, ui_type: None },
    SpecField { name: "verify_ssl", description: Some("Whether to verify SSL certificates. Defaults to True."), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(true)), ui_type: None },
] };

#[rustfmt::skip]
pub(super) const IBM_GUARDRAILS_PARAMS: &[SpecField] = &[
    SpecField { name: "block_on_detection", description: Some("Whether to block requests when detections are found. Defaults to True."), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(true)), ui_type: None },
    SpecField { name: "detector_params", description: Some("Dictionary of arguments to pass to the detector."), required: false, shape: SpecShape::Mapping { keys: &[], value: &SpecShape::Text }, default: Some(SpecValue::Null), ui_type: None },
    SpecField { name: "extra_headers", description: Some("Dictionary of extra headers to pass to the detector."), required: false, shape: SpecShape::Mapping { keys: &[], value: &SpecShape::Text }, default: Some(SpecValue::Null), ui_type: None },
    SpecField { name: "score_threshold", description: Some("Minimum score threshold to consider a detection as a violation (0.0 to 1.0). If set, detections below this threshold will be ignored."), required: false, shape: SpecShape::Number, default: None, ui_type: None },
];

#[rustfmt::skip]
pub(super) const MODEL_ARMOR: SpecModel = SpecModel { ui_friendly_name: Some("Google Cloud Model Armor"), fields: &[
    SpecField { name: "api_endpoint", description: Some("Optional custom API endpoint for Model Armor"), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "credentials", description: Some("Path to Google Cloud credentials JSON file or JSON string"), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "fail_on_error", description: Some("Whether to fail the request if Model Armor encounters an error"), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(true)), ui_type: None },
    SpecField { name: "location", description: Some("Google Cloud location/region (e.g., us-central1)"), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "project_id", description: Some("Google Cloud project ID"), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "sanitize_error_detail", description: Some("Omit the raw Model Armor response from caller-facing errors and logs by default. Set False to restore verbose output."), required: false, shape: SpecShape::Boolean, default: Some(SpecValue::Bool(true)), ui_type: None },
    SpecField { name: "template_id", description: Some("The ID of your Model Armor template"), required: false, shape: SpecShape::Text, default: None, ui_type: None },
] };

#[rustfmt::skip]
pub(super) const PANGEA: SpecModel = SpecModel { ui_friendly_name: Some("Pangea Guardrail"), fields: &[
    SpecField { name: "api_base", description: Some("The Pangea API base URL. Defaults to https://ai-guard.aws.us.pangea.cloud. Also checks if the PANGEA_API_BASE env var is set."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "api_key", description: Some("The Pangea API key. Reads from PANGEA_API_KEY env var if None."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "optional_params", description: Some("Optional parameters for the guardrail"), required: false, shape: SpecShape::Nested(PANGEA_PARAMS), default: None, ui_type: None },
] };

#[rustfmt::skip]
pub(super) const PANGEA_PARAMS: &[SpecField] = &[
    SpecField { name: "pangea_input_recipe", description: Some("The Pangea input recipe for the Pangea guardrail. Used for pre-call hook."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "pangea_output_recipe", description: Some("The Pangea output recipe for the Pangea guardrail. Used for post-call hook."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
];

#[rustfmt::skip]
pub(super) const SINGULR: SpecModel = SpecModel { ui_friendly_name: Some("Singulr"), fields: &[
    SpecField { name: "block_on_error", description: Some("Whether to block requests when the Singulr Guardrails API is unavailable or returns an error. If enabled, requests fail closed. If disabled, requests continue without guardrail enforcement (fail open)."), required: false, shape: SpecShape::Boolean, default: None, ui_type: None },
    SpecField { name: "singulr_api_base", description: Some("The Singulr API base URL. Get base URL from Singulr Platform."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "singulr_api_key", description: Some("The Singulr API key. Generate API key from Singulr Platform."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "singulr_application_id", description: Some("The Singulr application ID. Get application ID from Singulr Platform."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
    SpecField { name: "singulr_guardrail_id", description: Some("The Singulr Guardrail ID. Get guardrail ID from Singulr Platform."), required: false, shape: SpecShape::Text, default: None, ui_type: None },
] };
