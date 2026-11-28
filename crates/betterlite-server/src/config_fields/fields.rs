use betterlite_config::sections::settings::JsonValue;
use serde_json::Number;

// the pydantic default of a listed field: None for almost all of them, 1.0 and 30 for the two that declare one
#[derive(Clone, Copy)]
pub enum ListDefault {
    Null,
    Integer(i64),
    Float(f64),
}

impl ListDefault {
    #[must_use]
    pub fn value(self) -> JsonValue {
        match self {
            Self::Null => JsonValue::Null,
            Self::Integer(number) => JsonValue::Number(Number::from(number)),
            Self::Float(number) => {
                Number::from_f64(number).map_or(JsonValue::Null, JsonValue::Number)
            }
        }
    }
}

// name, GENERAL_SETTINGS_CONFIG_LIST_FIELD_TYPES kind, and the pydantic default
pub type ListField = (&'static str, &'static str, ListDefault);

// the listed fields, in the order ConfigGeneralSettings declares them
pub const LIST_FIELDS: [ListField; 34] = [
    (
        "max_in_flight_requests_per_worker",
        "Integer",
        ListDefault::Null,
    ),
    (
        "max_queued_requests_per_worker",
        "Integer",
        ListDefault::Null,
    ),
    (
        "admission_queue_timeout_seconds",
        "Float",
        ListDefault::Float(1.0),
    ),
    ("max_parallel_requests", "Integer", ListDefault::Null),
    ("global_max_parallel_requests", "Integer", ListDefault::Null),
    ("user_api_key_cache_max_size", "Integer", ListDefault::Null),
    ("max_request_size_mb", "Integer", ListDefault::Null),
    ("max_batch_file_size_mb", "Integer", ListDefault::Null),
    ("max_file_size_mb", "Integer", ListDefault::Null),
    ("allowed_file_extensions", "List", ListDefault::Null),
    ("blocked_file_extensions", "List", ListDefault::Null),
    ("max_response_size_mb", "Integer", ListDefault::Null),
    (
        "proxy_config_reload_interval_seconds",
        "Integer",
        ListDefault::Integer(30),
    ),
    ("cancel_on_disconnect", "Boolean", ListDefault::Null),
    ("pass_through_endpoints", "PydanticModel", ListDefault::Null),
    ("transcribe_media_buckets", "List", ListDefault::Null),
    ("store_prompts_in_spend_logs", "Boolean", ListDefault::Null),
    (
        "disable_auto_add_proxy_admin_to_teams",
        "Boolean",
        ListDefault::Null,
    ),
    (
        "maximum_spend_logs_retention_period",
        "String",
        ListDefault::Null,
    ),
    (
        "maximum_health_check_retention_period",
        "String",
        ListDefault::Null,
    ),
    (
        "maximum_daily_tag_spend_retention_period",
        "String",
        ListDefault::Null,
    ),
    (
        "maximum_spend_logs_cleanup_batch_size",
        "Integer",
        ListDefault::Null,
    ),
    (
        "maximum_spend_logs_cleanup_max_batches",
        "Integer",
        ListDefault::Null,
    ),
    (
        "maximum_spend_logs_cleanup_run_budget",
        "String",
        ListDefault::Null,
    ),
    (
        "maximum_spend_logs_cleanup_batch_timeout",
        "String",
        ListDefault::Null,
    ),
    ("mcp_internal_ip_ranges", "List", ListDefault::Null),
    ("mcp_allowed_clients", "TypedDictionary", ListDefault::Null),
    ("mcp_client_id_header", "String", ListDefault::Null),
    ("mcp_trusted_proxy_ranges", "List", ListDefault::Null),
    ("mcp_xff_num_trusted_hops", "Integer", ListDefault::Null),
    ("store_model_in_db", "Boolean", ListDefault::Null),
    (
        "forward_client_headers_to_llm_api",
        "Boolean",
        ListDefault::Null,
    ),
    ("mcp_required_fields", "List", ListDefault::Null),
    (
        "apply_user_budget_to_team_keys",
        "Boolean",
        ListDefault::Null,
    ),
];

// ConfigGeneralSettings.model_fields descriptions, in the same order as LIST_FIELDS
pub const LIST_DESCRIPTIONS: [&str; 34] = [
    "maximum concurrent requests handled by each worker",
    "maximum requests waiting for a worker slot",
    "maximum time a request waits for a worker slot",
    "maximum parallel requests for each api key",
    "global max parallel requests to allow for a proxy instance.",
    "max number of entries (virtual keys, teams, users, end users, memberships, ...) each worker keeps in its in-memory auth cache. Defaults to 200. Raise this if you have more active keys than that or auth lookups keep hitting the DB",
    "max request size in MB, if a request is larger than this size it will be rejected",
    "max batch input file size in MB for /v1/files uploads with purpose=batch, if a file is larger than this size it will be rejected before being forwarded to the provider",
    "max file size in MB for /v1/files uploads, for any purpose, if a file is larger than this size it will be rejected before being forwarded to the provider",
    "the only file extensions (e.g. ['.jsonl', '.pdf', '.txt']) accepted on /v1/files uploads, for any purpose, matched case-insensitively against the uploaded filename. Files with any other extension, or none, are rejected. An empty list rejects every upload. Unset means no allowlist is applied",
    "file extensions (e.g. ['.exe', '.sh']) rejected on /v1/files uploads, for any purpose, matched case-insensitively against the uploaded filename. Deprecated in favour of allowed_file_extensions; still enforced, after the allowlist, when set",
    "max response size in MB, if a response is larger than this size it will be rejected",
    "how often (in seconds) each pod reloads config-in-DB objects (models, credentials, guardrails, etc.) when store_model_in_db is enabled; lower values speed up multi-pod convergence at the cost of more DB load. Applied on proxy startup",
    "cancel the in-flight upstream LLM request (non-streaming) when the client disconnects, freeing backend capacity (e.g. a vLLM GPU slot); the request is logged as a 499 failure",
    "Set-up pass-through endpoints for provider-specific endpoints. Docs - https://docs.litellm.ai/docs/proxy/pass_through",
    "S3 bucket names that keys other than proxy admins may read media from and write transcripts to through the Amazon Transcribe pass-through. Unset means only proxy admins can start transcription jobs.",
    "If True, stores request messages and responses in spend logs. Default is False.",
    "By default, the user calling /team/new is automatically added to the new team as a team admin. If True, proxy admins are no longer auto-added; members explicitly listed in members_with_roles are unaffected. Default is False.",
    "Maximum retention period for spend logs (e.g., '7d' for 7 days). Logs older than this will be deleted.",
    "Maximum retention period for health-check rows (e.g., '30d'). Rows whose checked_at is older than this are deleted by the spend log cleanup job, on that job's schedule. Unset means rows are never deleted. Set this well above health_check_interval because /health and the UI read the latest row per model.",
    "Maximum retention period for per-day tag spend aggregate rows (e.g., '90d'). Rows whose day is older than this are deleted by the spend log cleanup job, on that job's schedule. Unset means rows are never deleted. Only historical tag usage analytics are affected; tag budgets read the lifetime counter.",
    "Rows deleted per DELETE statement by the spend log cleanup job. Defaults to 1000.",
    "Maximum DELETE statements the spend log cleanup job issues per table per run. Defaults to 500.",
    "Wall-clock budget for one spend log cleanup run (e.g. '5m'), shared across every table it prunes. A run that hits the budget stops and the next run resumes from where it left off. Defaults to '5m'.",
    "Postgres statement_timeout and lock_timeout applied to each spend log cleanup delete batch (e.g. '30s'), so cleanup cannot hold row locks or a connection indefinitely. Defaults to '30s'.",
    "Custom CIDR ranges that define internal/private networks for MCP access control. When set, only these ranges are treated as internal. Defaults to RFC 1918 private ranges (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 127.0.0.0/8).",
    "MCP client applications admitted by the gateway, each an {alias, value} pair where alias is the name shown in the dashboard and logs and value is the identity that must match exactly. When set, every MCP request must carry a client identity equal to one of the values: a JWT caller is identified by the claim named in litellm_jwtauth.mcp_client_id_jwt_field, any other caller by the header named in mcp_client_id_header. A request with no resolvable identity, or an unlisted one, is rejected with 403. Unset means every client is admitted.",
    "Request header whose value names the calling MCP client application (for example 'x-mcp-client') for callers that did not authenticate with a JWT, used only while mcp_allowed_clients is set. The client picks this value itself, so it is a policy control rather than a security boundary; prefer litellm_jwtauth.mcp_client_id_jwt_field where callers use JWTs.",
    "CIDR ranges of trusted reverse proxies. When set, X-Forwarded-For and X-Forwarded-* origin headers are only trusted from these IPs.",
    "Number of trusted reverse proxies/load balancers in front of the gateway that append to X-Forwarded-For. When set (and mcp_trusted_proxy_ranges validates the direct peer), the client IP for MCP access control is read this many entries from the right of the chain instead of the spoofable leftmost value, defeating append-style X-Forwarded-For forgery.",
    "If True, models and config are stored in and loaded from the database. Default is False.",
    "If True, forwards client headers (e.g. Authorization) to the LLM API. Required for Claude Code with Max subscription.",
    "List of MCP server fields that must be filled in for a submission to pass standards checks (e.g. ['description', 'source_url', 'alias']).",
    "If True, a user's personal max_budget is enforced on every request they make, including requests made with a team-scoped key. Defaults to False, where a team-scoped key is governed only by the team and team-member budgets and the key owner's personal max_budget does not apply (see GitHub issue #12905).",
];

// PassThroughGenericEndpoint.__annotations__: the name and the rendered type of every nested field, in declaration order
pub const NESTED_FIELDS: [(&str, &str); 12] = [
    ("id", "str | None"),
    ("path", "str"),
    ("target", "str"),
    ("headers", "dict"),
    ("default_query_params", "dict"),
    ("include_subpath", "bool"),
    ("cost_per_request", "float"),
    ("timeout", "float | None"),
    ("auth", "bool"),
    (
        "guardrails",
        "dict[str, litellm.proxy._types.PassThroughGuardrailSettings | None] | None",
    ),
    ("is_from_config", "bool"),
    ("methods", "list[str] | None"),
];

pub const NESTED_DESCRIPTIONS: [&str; 12] = [
    "Optional unique identifier for the pass-through endpoint. If not provided, endpoints will be identified by path for backwards compatibility.",
    "The route to be added to the LiteLLM Proxy Server.",
    "The URL to which requests for this path should be forwarded.",
    "Key-value pairs of headers to be forwarded with the request. You can set any key value pair here and it will be forwarded to your target endpoint",
    "Key-value pairs of default query parameters to be sent with every request to this endpoint. These can be overridden by client-provided query parameters. For example: {'key': 'default_value', 'api_version': '2023-01'}",
    "If True, requests to subpaths of the path will be forwarded to the target endpoint. For example, if the path is /bria and include_subpath is True, requests to /bria/v1/text-to-image/base/2.3 will be forwarded to the target endpoint.",
    "The USD cost per request to the target endpoint. This is used to calculate the cost of the request to the target endpoint.",
    "Upstream request timeout in seconds for this pass-through endpoint. If unset, uses general_settings.pass_through_request_timeout (default 600).",
    "Whether authentication is required for the pass-through endpoint. Defaults to True so a pass-through silently created without an explicit value still requires a valid LiteLLM API key \u{2014} set to False only if the endpoint is meant to be a public forwarder (e.g. an unauthenticated webhook target).",
    "Guardrails configuration for this passthrough endpoint. Dict keys are guardrail names, values are optional settings for field targeting. When set, all org/team/key level guardrails will also execute. Defaults to None (no guardrails execute).",
    "True if this endpoint is defined in the config file, False if from DB. Config-defined endpoints cannot be edited via the UI.",
    "List of HTTP methods this endpoint handles (e.g., ['GET', 'POST']). If None or empty, all methods (GET, POST, PUT, DELETE, PATCH) are supported for backward compatibility. This allows the same path to have different targets for different HTTP methods.",
];
