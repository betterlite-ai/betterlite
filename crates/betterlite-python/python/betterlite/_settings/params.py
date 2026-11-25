from typing import Protocol

from pydantic import JsonValue
from typing_extensions import override

from betterlite.types.guardrails.core import GuardrailItem
from betterlite.types.prompts.init_prompts import PromptSpec

email = None
token = None
bedrock_neutralize_orphaned_tool_blocks = True  # ast-grep-ignore: no-long-names
strip_anthropic_total_tokens = False
anthropic_sse_ping_interval_seconds = 15.0  # ast-grep-ignore: no-long-names
sse_keepalive_ping_interval_seconds = None  # ast-grep-ignore: no-long-names
retry = True
api_key = None
openai_key = None
groq_key = None
gigachat_key = None
xai_key = None
databricks_key = None
openai_like_key = None
azure_key = None
anthropic_key = None
replicate_key = None
bytez_key = None
gdc_key = None
gdc_api_base = None
cohere_key = None
infinity_key = None
clarifai_key = None
maritalk_key = None
ai21_key = None
ollama_key = None
openrouter_key = None
datarobot_key = None
predibase_key = None
huggingface_key = None
vertex_project = None
vertex_location = None
predibase_tenant_id = None
togetherai_api_key = None
cloudflare_api_key = None
vercel_ai_gateway_key = None
baseten_key = None
llama_api_key = None
aleph_alpha_key = None
nlp_cloud_key = None
novita_api_key = None
snowflake_key = None
gradient_ai_api_key = None
nebius_key = None
wandb_key = None
heroku_key = None
cometapi_key = None
ovhcloud_key = None
lemonade_key = None
sap_service_key = None
amazon_nova_api_key = None
inception_key = None
common_cloud_provider_auth_params = {  # ast-grep-ignore: no-long-names
    "params": ["project", "region_name", "token"],
    "providers": ["vertex_ai", "bedrock", "watsonx", "azure", "vertex_ai_beta"],
}
use_litellm_proxy = False
use_client = False
ssl_verify = True
ssl_security_level = None
ssl_certificate = None
user_url_validation = True
user_url_allowed_hosts: list[str] = []
provider_url_destination_allowed_hosts: list[str] = []  # ast-grep-ignore: no-long-names
otel_tenant_destination_mode = None
ssl_ecdh_curve = None
disable_streaming_logging = False
disable_token_counter = False
disable_add_transform_inline_image_block = False  # ast-grep-ignore: no-long-names
disable_add_user_agent_to_request_tags = False  # ast-grep-ignore: no-long-names
disable_anthropic_gemini_context_caching_transform = False  # ast-grep-ignore: no-long-names
openai_system_messages_first = False
disable_vertex_batch_output_transformation = False  # ast-grep-ignore: no-long-names
extra_spend_tag_headers = None
in_memory_llm_clients_cache: JsonValue = None
safe_memory_mode = False
enable_azure_ad_token_refresh = False
proxy_auth = None
AZURE_DEFAULT_API_VERSION = "2025-02-01-preview"
WATSONX_DEFAULT_API_VERSION = "2024-03-13"
COHERE_DEFAULT_EMBEDDING_INPUT_TYPE = "search_document"  # ast-grep-ignore: no-long-names
credential_list: list[JsonValue] = []
llamaguard_model_name = None
openai_moderations_model_name = None
presidio_ad_hoc_recognizers = None
google_moderation_confidence_threshold = None  # ast-grep-ignore: no-long-names
llamaguard_unsafe_content_categories = None  # ast-grep-ignore: no-long-names
blocked_user_list = None
banned_keywords_list = None
llm_guard_mode = "all"
guardrail_name_config_map: dict[str, GuardrailItem] = {}
include_cost_in_streaming_usage = False  # ast-grep-ignore: no-long-names
reasoning_auto_summary = False
prompt_name_config_map: dict[str, PromptSpec] = {}
enable_preview_features = False
return_response_headers = False
enable_json_schema_validation = False
enable_model_config_credential_overrides = False  # ast-grep-ignore: no-long-names
enable_key_alias_format_validation = False  # ast-grep-ignore: no-long-names
key_alias_pattern = None
enable_gemini_default_thinking_level_low = False  # ast-grep-ignore: no-long-names
logging = True
enable_loadbalancing_on_batch_endpoints = None  # ast-grep-ignore: no-long-names
require_managed_files = False
enable_caching_on_provider_specific_optional_params = False  # ast-grep-ignore: no-long-names
caching_with_models = False
class CacheReference(Protocol):
    @override
    def __str__(self) -> str: ...


cache: CacheReference | None = None
default_in_memory_ttl = None
default_redis_ttl = None
default_redis_batch_cache_expiry = None  # ast-grep-ignore: no-long-names
model_alias_map: dict[str, str] = {}
model_group_settings = None
max_budget = 0.0
budget_exceeded_status_code = 422
budget_duration = None
budget_exceeded_throttle_percentage = None  # ast-grep-ignore: no-long-names
forward_traceparent_to_llm_provider = False  # ast-grep-ignore: no-long-names
_current_cost = 0.0
error_logs: dict[str, JsonValue] = {}
add_function_to_prompt = False
client_session = None
aclient_session = None
model_fallbacks = None
suppress_debug_info = False
dynamodb_table_name = None
s3_callback_params = None
s3_audit_callback_params = None
datadog_llm_observability_params = None  # ast-grep-ignore: no-long-names
datadog_params = None
newrelic_params = None
pointfive_params = None
zerobus_params = None
aws_sqs_callback_params = None
generic_logger_headers = None
default_key_generate_params = None
default_key_max_budget_alert_emails = None  # ast-grep-ignore: no-long-names
upperbound_key_generate_params = None
key_generation_settings = None
default_internal_user_params = None
default_team_params = None
default_team_settings = None
max_user_budget = None
default_max_internal_user_budget = None  # ast-grep-ignore: no-long-names
max_internal_user_budget = None
max_ui_session_budget = 1.0
internal_user_budget_duration = None
budget_rollover = False
tag_budget_config = None
max_end_user_budget = None
max_end_user_budget_id = None
validate_end_user_id_in_db = False
block_requests_for_models_without_pricing = False  # ast-grep-ignore: no-long-names
disable_end_user_cost_tracking = None
disable_end_user_cost_tracking_prometheus_only = None  # ast-grep-ignore: no-long-names
enable_end_user_cost_tracking_prometheus_only = None  # ast-grep-ignore: no-long-names
custom_prometheus_metadata_labels: list[str] = []  # ast-grep-ignore: no-long-names
custom_prometheus_tags: list[str] = []
prometheus_metrics_config = None
prometheus_exclude_metrics = None
prometheus_exclude_labels = None
prometheus_emit_stream_label = False
# ast-grep-ignore: no-long-names
prometheus_deployment_and_latency_caller_identity = "api_key_alias"
prometheus_emit_input_sequence_length_label = False  # ast-grep-ignore: no-long-names
prometheus_emit_rate_limit_labels = False  # ast-grep-ignore: no-long-names
prometheus_user_budget_label_include_email_alias = False  # ast-grep-ignore: no-long-names
prometheus_end_user_metrics_max_series_per_metric = 10000  # ast-grep-ignore: no-long-names
prometheus_end_user_metrics_ttl_seconds = 3600.0  # ast-grep-ignore: no-long-names
prometheus_end_user_metrics_cleanup_interval_seconds = 60.0  # ast-grep-ignore: no-long-names
disable_add_prefix_to_prompt = False
disable_copilot_system_to_assistant = False  # ast-grep-ignore: no-long-names
public_mcp_servers = None
public_mcp_hub_strict_whitelist = True  # ast-grep-ignore: no-long-names
public_model_groups = None
public_skills_index = False
public_agent_groups = None
agent_search_embedding_model = None
mcp_tool_search = None
skill_search_embedding_model = None
public_model_groups_links: dict[str, JsonValue] = {}
priority_reservation = None
use_aiohttp_transport = True
aiohttp_trust_env = False
disable_aiohttp_transport = False
disable_aiohttp_trust_env = False
force_ipv4 = False
http2 = False
network_mock = False
disable_stop_sequence_limit = False
num_retries = None
max_fallbacks = None
default_fallbacks = None
fallbacks = None
context_window_fallbacks = None
content_policy_fallbacks = None
allowed_fails = 3
allow_dynamic_callback_disabling = True  # ast-grep-ignore: no-long-names
num_retries_per_request = None
secret_manager_client = None
_google_kms_resource_name = None
_key_management_system = None
output_parse_pii = False
check_provider_endpoint = False
api_base = None
headers = None
api_version = None
organization = None
project = None
config_path = None
vertex_ai_safety_settings = None


__all__ = [
    "AZURE_DEFAULT_API_VERSION", "COHERE_DEFAULT_EMBEDDING_INPUT_TYPE",
    "WATSONX_DEFAULT_API_VERSION", "_current_cost", "_google_kms_resource_name",
    "_key_management_system", "aclient_session", "add_function_to_prompt",
    "agent_search_embedding_model", "ai21_key", "aiohttp_trust_env", "aleph_alpha_key",
    "allow_dynamic_callback_disabling", "allowed_fails", "amazon_nova_api_key", "anthropic_key",
    "anthropic_sse_ping_interval_seconds", "api_base", "api_key", "api_version",
    "aws_sqs_callback_params", "azure_key", "banned_keywords_list", "baseten_key",
    "bedrock_neutralize_orphaned_tool_blocks", "block_requests_for_models_without_pricing",
    "blocked_user_list", "budget_duration", "budget_exceeded_status_code",
    "budget_exceeded_throttle_percentage", "budget_rollover", "bytez_key", "cache",
    "caching_with_models", "check_provider_endpoint", "clarifai_key", "client_session",
    "cloudflare_api_key", "cohere_key", "cometapi_key", "common_cloud_provider_auth_params",
    "config_path", "content_policy_fallbacks", "context_window_fallbacks", "credential_list",
    "custom_prometheus_metadata_labels", "custom_prometheus_tags", "databricks_key",
    "datadog_llm_observability_params", "datadog_params", "datarobot_key", "default_fallbacks",
    "default_in_memory_ttl", "default_internal_user_params", "default_key_generate_params",
    "default_key_max_budget_alert_emails", "default_max_internal_user_budget",
    "default_redis_batch_cache_expiry", "default_redis_ttl", "default_team_params",
    "default_team_settings", "disable_add_prefix_to_prompt",
    "disable_add_transform_inline_image_block", "disable_add_user_agent_to_request_tags",
    "disable_aiohttp_transport", "disable_aiohttp_trust_env",
    "disable_anthropic_gemini_context_caching_transform", "disable_copilot_system_to_assistant",
    "disable_end_user_cost_tracking", "disable_end_user_cost_tracking_prometheus_only",
    "disable_stop_sequence_limit", "disable_streaming_logging", "disable_token_counter",
    "disable_vertex_batch_output_transformation", "dynamodb_table_name", "email",
    "enable_azure_ad_token_refresh", "enable_caching_on_provider_specific_optional_params",
    "enable_end_user_cost_tracking_prometheus_only", "enable_gemini_default_thinking_level_low",
    "enable_json_schema_validation", "enable_key_alias_format_validation",
    "enable_loadbalancing_on_batch_endpoints", "enable_model_config_credential_overrides",
    "enable_preview_features", "error_logs", "extra_spend_tag_headers", "fallbacks", "force_ipv4",
    "forward_traceparent_to_llm_provider", "gdc_api_base", "gdc_key", "generic_logger_headers",
    "gigachat_key", "google_moderation_confidence_threshold", "gradient_ai_api_key", "groq_key",
    "guardrail_name_config_map", "headers", "heroku_key", "http2", "huggingface_key",
    "in_memory_llm_clients_cache", "inception_key", "include_cost_in_streaming_usage",
    "infinity_key", "internal_user_budget_duration", "key_alias_pattern", "key_generation_settings",
    "lemonade_key", "llama_api_key", "llamaguard_model_name",
    "llamaguard_unsafe_content_categories", "llm_guard_mode", "logging", "maritalk_key",
    "max_budget", "max_end_user_budget", "max_end_user_budget_id", "max_fallbacks",
    "max_internal_user_budget", "max_ui_session_budget", "max_user_budget", "mcp_tool_search",
    "model_alias_map", "model_fallbacks", "model_group_settings", "nebius_key", "network_mock",
    "newrelic_params", "nlp_cloud_key", "novita_api_key", "num_retries", "num_retries_per_request",
    "ollama_key", "openai_key", "openai_like_key", "openai_moderations_model_name",
    "openai_system_messages_first", "openrouter_key", "organization",
    "otel_tenant_destination_mode", "output_parse_pii", "ovhcloud_key", "pointfive_params",
    "predibase_key", "predibase_tenant_id", "presidio_ad_hoc_recognizers", "priority_reservation",
    "project", "prometheus_deployment_and_latency_caller_identity",
    "prometheus_emit_input_sequence_length_label", "prometheus_emit_rate_limit_labels",
    "prometheus_emit_stream_label", "prometheus_end_user_metrics_cleanup_interval_seconds",
    "prometheus_end_user_metrics_max_series_per_metric", "prometheus_end_user_metrics_ttl_seconds",
    "prometheus_exclude_labels", "prometheus_exclude_metrics", "prometheus_metrics_config",
    "prometheus_user_budget_label_include_email_alias", "prompt_name_config_map",
    "provider_url_destination_allowed_hosts", "proxy_auth", "public_agent_groups",
    "public_mcp_hub_strict_whitelist", "public_mcp_servers", "public_model_groups",
    "public_model_groups_links", "public_skills_index", "reasoning_auto_summary", "replicate_key",
    "require_managed_files", "retry", "return_response_headers", "s3_audit_callback_params",
    "s3_callback_params", "safe_memory_mode", "sap_service_key", "secret_manager_client",
    "skill_search_embedding_model", "snowflake_key", "sse_keepalive_ping_interval_seconds",
    "ssl_certificate", "ssl_ecdh_curve", "ssl_security_level", "ssl_verify",
    "strip_anthropic_total_tokens", "suppress_debug_info", "tag_budget_config",
    "togetherai_api_key", "token", "upperbound_key_generate_params", "use_aiohttp_transport",
    "use_client", "use_litellm_proxy", "user_url_allowed_hosts", "user_url_validation",
    "validate_end_user_id_in_db", "vercel_ai_gateway_key", "vertex_ai_safety_settings",
    "vertex_location", "vertex_project", "wandb_key", "xai_key", "zerobus_params",
]
