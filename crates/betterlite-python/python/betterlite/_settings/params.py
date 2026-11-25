from collections.abc import Mapping
from typing import Protocol

import httpx2
from pydantic import JsonValue
from typing_extensions import override

from betterlite._router_types_routing import ModelGroupSettings
from betterlite.types.guardrails.core import GuardrailItem
from betterlite.types.integrations import datadog, newrelic, pointfive, zerobus
from betterlite.types.integrations.datadog_llm_obs import DatadogLLMObsInitParams
from betterlite.types.prompts.init_prompts import PromptSpec
from betterlite.types.proxy.management_endpoints import ui_sso
from betterlite.types.secret_managers.main import KeyManagementSystem
from betterlite.types.utils import BudgetConfig

class CacheReference(Protocol):
    @override
    def __str__(self) -> str: ...

class ProxyAuthReference(Protocol):
    def get_auth_headers(self) -> Mapping[str, str]: ...

email: str | None = None
token: str | None = None
bedrock_neutralize_orphaned_tool_blocks = True  # ast-grep-ignore: no-long-names
strip_anthropic_total_tokens = False
anthropic_sse_ping_interval_seconds = 15.0  # ast-grep-ignore: no-long-names
sse_keepalive_ping_interval_seconds: float | None = None  # ast-grep-ignore: no-long-names
retry = True
api_key: str | None = None
openai_key: str | None = None
groq_key: str | None = None
gigachat_key: str | None = None
xai_key: str | None = None
databricks_key: str | None = None
openai_like_key: str | None = None
azure_key: str | None = None
anthropic_key: str | None = None
replicate_key: str | None = None
bytez_key: str | None = None
gdc_key: str | None = None
gdc_api_base: str | None = None
cohere_key: str | None = None
infinity_key: str | None = None
clarifai_key: str | None = None
maritalk_key: str | None = None
ai21_key: str | None = None
ollama_key: str | None = None
openrouter_key: str | None = None
datarobot_key: str | None = None
predibase_key: str | None = None
huggingface_key: str | None = None
vertex_project: str | None = None
vertex_location: str | None = None
predibase_tenant_id: str | None = None
togetherai_api_key: str | None = None
cloudflare_api_key: str | None = None
vercel_ai_gateway_key: str | None = None
baseten_key: str | None = None
llama_api_key: str | None = None
aleph_alpha_key: str | None = None
nlp_cloud_key: str | None = None
novita_api_key: str | None = None
snowflake_key: str | None = None
gradient_ai_api_key: str | None = None
nebius_key: str | None = None
wandb_key: str | None = None
heroku_key: str | None = None
cometapi_key: str | None = None
ovhcloud_key: str | None = None
lemonade_key: str | None = None
sap_service_key: str | None = None
amazon_nova_api_key: str | None = None
inception_key: str | None = None
common_cloud_provider_auth_params = {  # ast-grep-ignore: no-long-names
    "params": ["project", "region_name", "token"],
    "providers": ["vertex_ai", "bedrock", "watsonx", "azure", "vertex_ai_beta"],
}
use_litellm_proxy = False
use_client = False
ssl_verify = True
ssl_security_level: str | None = None
ssl_certificate: str | None = None
user_url_validation = True
user_url_allowed_hosts: list[str] = []
provider_url_destination_allowed_hosts: list[str] = []  # ast-grep-ignore: no-long-names
otel_tenant_destination_mode: str | None = None
ssl_ecdh_curve: str | None = None
disable_streaming_logging = False
disable_token_counter = False
disable_add_transform_inline_image_block = False  # ast-grep-ignore: no-long-names
disable_add_user_agent_to_request_tags = False  # ast-grep-ignore: no-long-names
disable_anthropic_gemini_context_caching_transform = False  # ast-grep-ignore: no-long-names
openai_system_messages_first = False
disable_vertex_batch_output_transformation = False  # ast-grep-ignore: no-long-names
extra_spend_tag_headers: list[str] | None = None
in_memory_llm_clients_cache: JsonValue = None
safe_memory_mode = False
enable_azure_ad_token_refresh = False
proxy_auth: ProxyAuthReference | None = None
AZURE_DEFAULT_API_VERSION = "2025-02-01-preview"
WATSONX_DEFAULT_API_VERSION = "2024-03-13"
COHERE_DEFAULT_EMBEDDING_INPUT_TYPE = "search_document"  # ast-grep-ignore: no-long-names
credential_list: list[JsonValue] = []
llamaguard_model_name: str | None = None
openai_moderations_model_name: str | None = None
presidio_ad_hoc_recognizers: str | None = None
google_moderation_confidence_threshold: float | None = None  # ast-grep-ignore: no-long-names
llamaguard_unsafe_content_categories: str | None = None  # ast-grep-ignore: no-long-names
blocked_user_list: str | list[JsonValue] | None = None
banned_keywords_list: str | list[JsonValue] | None = None
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
key_alias_pattern: str | None = None
enable_gemini_default_thinking_level_low = False  # ast-grep-ignore: no-long-names
logging = True
enable_loadbalancing_on_batch_endpoints: bool | None = None  # ast-grep-ignore: no-long-names
require_managed_files = False
enable_caching_on_provider_specific_optional_params = False  # ast-grep-ignore: no-long-names
caching_with_models = False
cache: CacheReference | None = None
default_in_memory_ttl: float | None = None
default_redis_ttl: float | None = None
default_redis_batch_cache_expiry: float | None = None  # ast-grep-ignore: no-long-names
model_alias_map: dict[str, str] = {}
model_group_settings: ModelGroupSettings | None = None
max_budget = 0.0
budget_exceeded_status_code = 422
budget_duration: str | None = None
budget_exceeded_throttle_percentage: float | None = None  # ast-grep-ignore: no-long-names
forward_traceparent_to_llm_provider = False  # ast-grep-ignore: no-long-names
_current_cost = 0.0
error_logs: dict[str, JsonValue] = {}
add_function_to_prompt = False
client_session: httpx2.Client | None = None
aclient_session: httpx2.AsyncClient | None = None
model_fallbacks: list[JsonValue] | None = None
suppress_debug_info = False
dynamodb_table_name: str | None = None
s3_callback_params: dict[str, JsonValue] | None = None
s3_audit_callback_params: dict[str, JsonValue] | None = None
# ast-grep-ignore: no-long-names
datadog_llm_observability_params: DatadogLLMObsInitParams | dict[str, JsonValue] | None = None
datadog_params: datadog.DatadogInitParams | dict[str, JsonValue] | None = None
newrelic_params: newrelic.NewRelicInitParams | dict[str, JsonValue] | None = None
pointfive_params: pointfive.PointFiveInitParams | Mapping[str, JsonValue] | None = None
zerobus_params: zerobus.ZerobusInitParams | Mapping[str, JsonValue] | None = None
aws_sqs_callback_params: dict[str, JsonValue] | None = None
generic_logger_headers: dict[str, JsonValue] | None = None
default_key_generate_params: dict[str, JsonValue] | None = None
# ast-grep-ignore: no-long-names
default_key_max_budget_alert_emails: dict[str, list[JsonValue]] | None = None
upperbound_key_generate_params: ui_sso.UpperboundKeyGenerateParams | None = None
key_generation_settings: object | None = None  # ast-grep-ignore: no-loose-types
default_internal_user_params: dict[str, JsonValue] | None = None
default_team_params: ui_sso.DefaultTeamSSOParams | dict[str, JsonValue] | None = None
default_team_settings: list[JsonValue] | None = None
max_user_budget: float | None = None
default_max_internal_user_budget: float | None = None  # ast-grep-ignore: no-long-names
max_internal_user_budget: float | None = None
max_ui_session_budget = 1.0
internal_user_budget_duration: str | None = None
budget_rollover = False
tag_budget_config: dict[str, BudgetConfig] | None = None
max_end_user_budget: float | None = None
max_end_user_budget_id: str | None = None
validate_end_user_id_in_db = False
block_requests_for_models_without_pricing = False  # ast-grep-ignore: no-long-names
disable_end_user_cost_tracking: bool | None = None
disable_end_user_cost_tracking_prometheus_only: bool | None = None  # ast-grep-ignore: no-long-names
enable_end_user_cost_tracking_prometheus_only: bool | None = None  # ast-grep-ignore: no-long-names
custom_prometheus_metadata_labels: list[str] = []  # ast-grep-ignore: no-long-names
custom_prometheus_tags: list[str] = []
prometheus_metrics_config: list[JsonValue] | None = None
prometheus_exclude_metrics: list[str] | None = None
prometheus_exclude_labels: list[str] | None = None
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
public_mcp_servers: list[str] | None = None
public_mcp_hub_strict_whitelist = True  # ast-grep-ignore: no-long-names
public_model_groups: list[str] | None = None
public_skills_index = False
public_agent_groups: list[str] | None = None
agent_search_embedding_model: str | None = None
mcp_tool_search: Mapping[str, JsonValue] | None = None
skill_search_embedding_model: str | None = None
public_model_groups_links: dict[str, JsonValue] = {}
priority_reservation: dict[str, object] | None = None  # ast-grep-ignore: no-loose-types
use_aiohttp_transport = True
aiohttp_trust_env = False
disable_aiohttp_transport = False
disable_aiohttp_trust_env = False
force_ipv4 = False
http2 = False
network_mock = False
disable_stop_sequence_limit = False
num_retries: int | None = None
max_fallbacks: int | None = None
default_fallbacks: list[JsonValue] | None = None
fallbacks: list[JsonValue] | None = None
context_window_fallbacks: list[JsonValue] | None = None
content_policy_fallbacks: list[JsonValue] | None = None
allowed_fails = 3
allow_dynamic_callback_disabling = True  # ast-grep-ignore: no-long-names
num_retries_per_request: int | None = None
secret_manager_client: object | None = None  # ast-grep-ignore: no-loose-types
_google_kms_resource_name: str | None = None
_key_management_system: KeyManagementSystem | None = None
output_parse_pii = False
check_provider_endpoint = False
api_base: str | None = None
headers = None
api_version: str | None = None
organization = None
project = None
config_path = None
vertex_ai_safety_settings: dict[str, JsonValue] | None = None

__all__ = ["AZURE_DEFAULT_API_VERSION", "COHERE_DEFAULT_EMBEDDING_INPUT_TYPE",
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
    "vertex_location", "vertex_project", "wandb_key", "xai_key", "zerobus_params"]
