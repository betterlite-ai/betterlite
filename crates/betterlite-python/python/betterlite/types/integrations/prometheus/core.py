import re
from collections.abc import Mapping, Sequence
from typing import Final, Literal

from pydantic import JsonValue

_INVALID_LABEL_NAME_CHARACTERS: Final = re.compile(r"[^a-zA-Z0-9_]")
_LABEL_NAME_FIRST_CHARACTER: Final = re.compile(r"[a-zA-Z_]")
# ast-grep-ignore: no-long-names
_PROMETHEUS_LABEL_VALUE_TRANSLATE_V1: Final = str.maketrans("\n", " ", "\r\u2028\u2029")

REQUESTED_MODEL: Final = "requested_model"
EXCEPTION_STATUS: Final = "exception_status"
EXCEPTION_CLASS: Final = "exception_class"
RATE_LIMIT_CATEGORY: Final = "rate_limit_category"
RATE_LIMIT_TYPE: Final = "rate_limit_type"
ZERO_COST_REASON_LABEL: Final = "reason"
STATUS_CODE: Final = "status_code"
EXCEPTION_LABELS: Final[list[str]] = [EXCEPTION_STATUS, EXCEPTION_CLASS]
LATENCY_BUCKETS: Final[tuple[float, ...]] = (
    0.005,
    0.01,
    0.025,
    0.05,
    0.1,
    0.25,
    0.5,
    1.0,
    2.0,
    5.0,
    10.0,
    30.0,
    60.0,
    120.0,
    300.0,
    420.0,
    600.0,
    float("inf"),
)

UNKNOWN_INPUT_SEQUENCE_LENGTH: Final = "unknown"
INPUT_SEQUENCE_LENGTH_BUCKETS: Final[tuple[tuple[int | float, str], ...]] = (
    (1_000, "0-1k"),
    (4_000, "1k-4k"),
    (16_000, "4k-16k"),
    (64_000, "16k-64k"),
    (float("inf"), "64k+"),
)

BATCH_DURATION_BUCKETS: Final[tuple[float, ...]] = (
    60.0,
    120.0,
    300.0,
    600.0,
    900.0,
    1800.0,
    3600.0,
    7200.0,
    14400.0,
    28800.0,
    43200.0,
    86400.0,
    float("inf"),
)


# ast-grep-ignore: no-long-names
def _sanitize_prometheus_label_name(label: str) -> str:
    if not label:
        return "_"
    sanitized = _INVALID_LABEL_NAME_CHARACTERS.sub("_", label)
    if sanitized and _LABEL_NAME_FIRST_CHARACTER.match(sanitized) is None:
        sanitized = "_" + sanitized
    if not sanitized:
        sanitized = "_"
    return sanitized


# ast-grep-ignore: no-long-names
def _sanitize_prometheus_label_value(value: JsonValue) -> str | None:
    if value is None:
        return None
    # ast-grep-ignore: no-long-names
    cleaned = str(value).translate(_PROMETHEUS_LABEL_VALUE_TRANSLATE_V1)
    return cleaned.replace("\\", "\\\\").replace('"', '\\"')


# ast-grep-ignore: no-long-names
def get_input_sequence_length_bucket(prompt_tokens: JsonValue) -> str:
    match prompt_tokens:
        case bool() | Mapping() | Sequence() | float() | None:
            return UNKNOWN_INPUT_SEQUENCE_LENGTH
        case int() as token_count:
            if token_count < 0:
                return UNKNOWN_INPUT_SEQUENCE_LENGTH
            return next(
                label for upper, label in INPUT_SEQUENCE_LENGTH_BUCKETS if token_count < upper
            )


DEFINED_PROMETHEUS_METRICS = Literal[
    "litellm_llm_api_latency_metric",
    "litellm_llm_api_time_to_first_token_metric",
    "litellm_request_total_latency_metric",
    "litellm_overhead_latency_metric",
    "litellm_overhead_with_guardrails_latency_metric",
    "litellm_remaining_requests_metric",
    "litellm_remaining_tokens_metric",
    "litellm_proxy_total_requests_metric",
    "litellm_proxy_failed_requests_metric",
    "litellm_deployment_latency_per_output_token",
    "litellm_requests_metric",
    "litellm_spend_metric",
    "litellm_total_tokens_metric",
    "litellm_input_tokens_metric",
    "litellm_output_tokens_metric",
    "litellm_input_cached_tokens_metric",
    "litellm_input_cache_creation_tokens_metric",
    "litellm_input_audio_tokens_metric",
    "litellm_output_reasoning_tokens_metric",
    "litellm_output_audio_tokens_metric",
    "litellm_video_duration_seconds_metric",
    "litellm_images_generated_metric",
    "litellm_deployment_successful_fallbacks",
    "litellm_deployment_failed_fallbacks",
    "litellm_remaining_team_budget_metric",
    "litellm_team_max_budget_metric",
    "litellm_team_budget_remaining_hours_metric",
    "litellm_team_members_metric",
    "litellm_remaining_org_budget_metric",
    "litellm_org_max_budget_metric",
    "litellm_org_budget_remaining_hours_metric",
    "litellm_remaining_api_key_budget_metric",
    "litellm_api_key_max_budget_metric",
    "litellm_api_key_budget_remaining_hours_metric",
    "litellm_remaining_user_budget_metric",
    "litellm_user_max_budget_metric",
    "litellm_user_budget_remaining_hours_metric",
    "litellm_remaining_customer_budget_metric",
    "litellm_customer_max_budget_metric",
    "litellm_customer_budget_remaining_hours_metric",
    "litellm_deployment_state",
    "litellm_deployment_failure_responses",
    "litellm_deployment_total_requests",
    "litellm_deployment_success_responses",
    "litellm_deployment_cooled_down",
    "litellm_pod_lock_manager_size",
    "litellm_in_memory_daily_spend_update_queue_size",
    "litellm_redis_daily_spend_update_queue_size",
    "litellm_in_memory_spend_update_queue_size",
    "litellm_redis_spend_update_queue_size",
    "litellm_request_queue_time_seconds",
    "litellm_guardrail_latency_seconds",
    "litellm_guardrail_errors_total",
    "litellm_guardrail_requests_total",
    "litellm_zero_cost_requests_total",
    "litellm_spend_capture_rate",
    "litellm_cache_hits_metric",
    "litellm_cache_misses_metric",
    "litellm_cached_tokens_metric",
    "litellm_provider_cache_read_input_tokens_metric",
    "litellm_provider_cache_creation_input_tokens_metric",
    "litellm_deployment_tpm_limit",
    "litellm_deployment_rpm_limit",
    "litellm_remaining_api_key_requests_for_model",
    "litellm_remaining_api_key_tokens_for_model",
    "litellm_api_key_rate_limit_allowed_metric",
    "litellm_api_key_rate_limit_used_metric",
    "litellm_team_rate_limit_allowed_metric",
    "litellm_team_rate_limit_used_metric",
    "litellm_llm_api_failed_requests_metric",
    "litellm_callback_logging_failures_metric",
    "litellm_in_flight_requests",
    "litellm_managed_batch_created_total",
    "litellm_managed_file_size_bytes",
    "litellm_managed_batch_duration_seconds",
    "litellm_managed_file_created_total",
    "litellm_managed_file_deleted_total",
    "litellm_check_batch_cost_jobs_polled",
    "litellm_check_batch_cost_jobs_processed_total",
    "litellm_check_batch_cost_errors_total",
    "litellm_check_batch_cost_last_run_timestamp",
    "litellm_mcp_tool_calls_total",
    "litellm_mcp_tool_call_spend_metric",
]

# ast-grep-ignore: no-long-names
PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS: Final[frozenset[str]] = frozenset(
    {
        "litellm_deployment_total_requests",
        "litellm_deployment_success_responses",
        "litellm_deployment_failure_responses",
        "litellm_request_total_latency_metric",
        "litellm_llm_api_latency_metric",
        "litellm_llm_api_time_to_first_token_metric",
        "litellm_request_queue_time_seconds",
        "litellm_overhead_latency_metric",
        "litellm_deployment_latency_per_output_token",
    }
)

# ast-grep-ignore: no-long-names
PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_VALUES: Final[tuple[str, ...]] = (
    "api_key_alias",
    "user_email",
    "both",
)

__all__ = [
    "BATCH_DURATION_BUCKETS",
    "DEFINED_PROMETHEUS_METRICS",
    "EXCEPTION_CLASS",
    "EXCEPTION_LABELS",
    "EXCEPTION_STATUS",
    "INPUT_SEQUENCE_LENGTH_BUCKETS",
    "LATENCY_BUCKETS",
    "PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_METRICS",
    "PROMETHEUS_DEPLOYMENT_AND_LATENCY_CALLER_IDENTITY_VALUES",
    "RATE_LIMIT_CATEGORY",
    "RATE_LIMIT_TYPE",
    "REQUESTED_MODEL",
    "STATUS_CODE",
    "UNKNOWN_INPUT_SEQUENCE_LENGTH",
    "ZERO_COST_REASON_LABEL",
    "_PROMETHEUS_LABEL_VALUE_TRANSLATE_V1",
    "_sanitize_prometheus_label_name",
    "_sanitize_prometheus_label_value",
    "get_input_sequence_length_bucket",
]
