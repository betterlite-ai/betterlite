from typing import Literal, get_args

from pydantic import JsonValue

CALLBACK_TYPES: JsonValue = None
input_callback: list[JsonValue] = []
success_callback: list[JsonValue] = []
failure_callback: list[JsonValue] = []
service_callback: list[JsonValue] = []
audit_log_callbacks: list[JsonValue] = []
_CALLBACK_NAMES = Literal[
    "lago", "openmeter", "logfire", "literalai", "litellm_agent", "dynamic_rate_limiter",
    "dynamic_rate_limiter_v3", "langsmith", "prometheus", "otel", "datadog", "datadog_metrics",
    "datadog_llm_observability", "galileo", "braintrust", "arize", "arize_phoenix", "langtrace",
    "gcs_bucket", "azure_storage", "opik", "argilla", "mlflow", "langfuse", "langfuse_otel",
    "weave_otel", "pagerduty", "humanloop", "azure_sentinel", "gcs_pubsub", "agentops",
    "anthropic_cache_control_hook", "generic_api", "resend_email", "sendgrid_email", "smtp_email",
    "deepeval", "s3_v2", "clickhouse", "pointfive", "zerobus", "aws_sqs",
    "vector_store_pre_call_hook", "dotprompt", "bitbucket", "gitlab", "cloudzero", "focus",
    "mavvrik", "vantage", "posthog", "levo", "compression_interception", "newrelic", "signoz",
]
# ast-grep-ignore: no-long-names
_custom_logger_compatible_callbacks_literal = _CALLBACK_NAMES
cold_storage_custom_logger = None
logged_real_time_event_types = None
_callback_name_args: tuple[str, ...] = get_args(_CALLBACK_NAMES)
# ast-grep-ignore: no-long-names
_known_custom_logger_compatible_callbacks: list[str] = list(_callback_name_args)
callbacks: list[JsonValue] = []
callback_settings: dict[str, dict[str, JsonValue]] = {}
initialized_langfuse_clients = 0
langfuse_default_tags = None
langfuse_enable_update_trace_keys = False  # ast-grep-ignore: no-long-names
langsmith_batch_size = None
prometheus_initialize_budget_metrics = False  # ast-grep-ignore: no-long-names
prometheus_latency_buckets = None
require_auth_for_metrics_endpoint = True  # ast-grep-ignore: no-long-names
argilla_batch_size = None
datadog_use_v1 = False
gcs_pub_sub_use_v1 = False
generic_api_use_v1 = False
argilla_transformation_object = None
_async_input_callback: list[JsonValue] = []
_async_success_callback: list[JsonValue] = []
_async_failure_callback: list[JsonValue] = []
pre_call_rules: list[JsonValue] = []
post_call_rules: list[JsonValue] = []
turn_off_message_logging = False
standard_logging_payload_excluded_fields = None  # ast-grep-ignore: no-long-names
log_raw_request_response = False
log_client_error_tracebacks = False
log_auth_failure_key_identity = False
request_correlation_in_logs = False
redact_messages_in_exceptions = False
redact_user_api_key_info = False
expose_router_debug_in_errors = True
filter_invalid_headers = False
add_user_information_to_llm_headers = None  # ast-grep-ignore: no-long-names
overwrite_user_with_key_hash = False
bedrock_request_metadata_fields = None  # ast-grep-ignore: no-long-names
store_audit_logs = None
skip_system_message_in_guardrail = False  # ast-grep-ignore: no-long-names
skip_tool_message_in_guardrail = False


__all__ = [
    "CALLBACK_TYPES", "_async_failure_callback", "_async_input_callback", "_async_success_callback",
    "_custom_logger_compatible_callbacks_literal", "_known_custom_logger_compatible_callbacks",
    "add_user_information_to_llm_headers", "argilla_batch_size", "argilla_transformation_object",
    "audit_log_callbacks", "bedrock_request_metadata_fields", "callback_settings", "callbacks",
    "cold_storage_custom_logger", "datadog_use_v1", "expose_router_debug_in_errors",
    "failure_callback", "filter_invalid_headers", "gcs_pub_sub_use_v1", "generic_api_use_v1",
    "initialized_langfuse_clients", "input_callback", "langfuse_default_tags",
    "langfuse_enable_update_trace_keys", "langsmith_batch_size", "log_auth_failure_key_identity",
    "log_client_error_tracebacks", "log_raw_request_response", "logged_real_time_event_types",
    "overwrite_user_with_key_hash", "post_call_rules", "pre_call_rules",
    "prometheus_initialize_budget_metrics", "prometheus_latency_buckets",
    "redact_messages_in_exceptions", "redact_user_api_key_info", "request_correlation_in_logs",
    "require_auth_for_metrics_endpoint", "service_callback", "skip_system_message_in_guardrail",
    "skip_tool_message_in_guardrail", "standard_logging_payload_excluded_fields",
    "store_audit_logs", "success_callback", "turn_off_message_logging",
]
