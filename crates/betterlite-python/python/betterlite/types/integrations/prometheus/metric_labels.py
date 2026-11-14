from typing import ClassVar, Final

from betterlite.types.integrations.prometheus.core import (
    DEFINED_PROMETHEUS_METRICS,
    _sanitize_prometheus_label_name,
)
from betterlite.types.integrations.prometheus.labels import UserAPIKeyLabelNames
from betterlite.types.integrations.prometheus.settings import PrometheusLabelSettings
from betterlite.types.integrations.prometheus.validation import (
    _resolve_deployment_and_latency_caller_identity_labels,
)

_EMPTY: Final[list[str]] = []
_PROXY_TOTAL_REQUESTS: Final = "litellm_proxy_total_requests_metric"
_PROXY_FAILED_REQUESTS: Final = "litellm_proxy_failed_requests_metric"
_USER_BUDGET_METRICS: Final[frozenset[str]] = frozenset({"litellm_remaining_user_budget_metric",
    "litellm_user_max_budget_metric", "litellm_user_budget_remaining_hours_metric"})
_ORG_LABEL_METRICS: Final[frozenset[str]] = frozenset({"litellm_llm_api_latency_metric",
    "litellm_llm_api_time_to_first_token_metric", "litellm_request_total_latency_metric",
    "litellm_request_queue_time_seconds", "litellm_proxy_total_requests_metric",
    "litellm_proxy_failed_requests_metric", "litellm_deployment_latency_per_output_token",
    "litellm_requests_metric", "litellm_spend_metric", "litellm_input_tokens_metric",
    "litellm_total_tokens_metric", "litellm_output_tokens_metric",
    "litellm_video_duration_seconds_metric", "litellm_images_generated_metric"})
_INPUT_SEQUENCE_LENGTH_METRICS: Final[frozenset[str]] = frozenset({"litellm_llm_api_latency_metric",
    "litellm_llm_api_time_to_first_token_metric", "litellm_request_total_latency_metric"})
_LLM_API_LATENCY_LABELS: Final[list[str]] = ["model", "hashed_api_key", "api_key_alias", "team",
    "team_alias", "requested_model", "end_user", "user", "model_id", "api_provider", "service_tier"]
_REQUEST_TOTAL_LATENCY_LABELS: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias",
    "requested_model", "team", "team_alias", "user", "model", "model_id", "api_provider",
    "service_tier"]
_QUEUE_TIME: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias", "requested_model",
    "team", "team_alias", "user", "model", "model_id", "api_provider"]
_PROXY_TOTAL: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias", "requested_model",
    "team", "team_alias", "user", "status_code", "user_email", "route", "client_ip", "user_agent",
    "model_id", "api_provider"]
_PROXY_FAILED_LABELS: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias",
    "requested_model", "team", "team_alias", "user", "user_email", "exception_status",
    "exception_class", "route", "client_ip", "user_agent", "model_id", "api_provider"]
_OUTPUT_TOKEN_LATENCY_LABELS: Final[list[str]] = ["litellm_model_name", "model_id", "api_base",
    "api_provider", "hashed_api_key", "api_key_alias", "team", "team_alias"]
_DEPLOYMENT_LIMIT_LABELS: Final[list[str]] = ["model_group", "api_provider", "api_base",
    "litellm_model_name", "hashed_api_key", "api_key_alias", "model_id"]
_REQUESTS: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias", "model", "team",
    "team_alias", "user", "user_email", "client_ip", "user_agent", "requested_model", "model_id",
    "api_provider"]
_SPEND: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias", "model", "team",
    "team_alias", "user", "user_email", "client_ip", "user_agent", "requested_model", "model_id",
    "api_provider", "service_tier"]
_ZERO_COST: Final[tuple[str, ...]] = ("requested_model", "model", "model_id", "api_provider",
    "reason")
_SPEND_CAPTURE_RATE_LABELS: Final[tuple[str, ...]] = ("api_provider",)
_TOKENS: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias", "model", "team",
    "team_alias", "user", "user_email", "requested_model", "model_id", "api_provider"]
_DEPLOYMENT_STATE_LABELS: Final[list[str]] = ["litellm_model_name", "model_id", "api_base",
    "api_provider"]
_DEPLOYMENT_RATE_LIMIT_LABELS: Final[list[str]] = ["model_group", "litellm_model_name", "model_id",
    "api_base", "api_provider"]
_COOLDOWN: Final[list[str]] = ["litellm_model_name", "model_id", "api_base", "api_provider",
    "exception_status"]
_FALLBACK: Final[list[str]] = ["requested_model", "fallback_model", "hashed_api_key",
    "api_key_alias", "team", "team_alias", "exception_status", "exception_class", "model_id"]
_TEAM_BUDGET: Final[list[str]] = ["team", "team_alias"]
_ORG_BUDGET: Final[list[str]] = ["org_id", "org_alias"]
_KEY_BUDGET: Final[list[str]] = ["hashed_api_key", "api_key_alias"]
_USER_BUDGET: Final[list[str]] = ["user"]
_CUSTOMER_BUDGET_LABELS: Final[tuple[str, ...]] = ("end_user",)
_MODEL_LIMITS: Final[list[str]] = ["hashed_api_key", "api_key_alias", "model", "model_id"]
_CALLBACK: Final[list[str]] = ["callback_name"]
_FAIL_RESPONSES: Final[list[str]] = ["model_group", "requested_model", "litellm_model_name",
    "model_id", "api_base", "api_provider", "exception_status", "exception_class", "hashed_api_key",
    "api_key_alias", "team", "team_alias", "client_ip", "user_agent"]
_DEPLOYMENT_TOTAL_LABELS: Final[list[str]] = ["model_group", "requested_model",
    "litellm_model_name", "model_id", "api_base", "api_provider", "hashed_api_key", "api_key_alias",
    "team", "team_alias", "client_ip", "user_agent"]
_KEY_RATE: Final[tuple[str, ...]] = ("hashed_api_key", "api_key_alias", "rate_limit_type")
_TEAM_RATE: Final[tuple[str, ...]] = ("team", "team_alias", "rate_limit_type")
_FAILED_API: Final[list[str]] = ["end_user", "hashed_api_key", "api_key_alias", "model", "team",
    "team_alias", "user", "model_id"]
_CACHE: Final[list[str]] = ["model", "hashed_api_key", "api_key_alias", "team", "team_alias",
    "end_user", "user", "model_id", "api_provider"]
_BATCH: Final[list[str]] = ["model", "api_provider", "user", "user_email", "api_key_alias"]
_BATCH_TIME: Final[list[str]] = ["model", "api_provider"]
_MCP_TOOL_CALL_LABELS: Final[list[str]] = ["mcp_tool_name", "mcp_server_name", "hashed_api_key",
    "api_key_alias", "team", "team_alias", "user", "end_user"]
_DEFAULT_SETTINGS: Final = PrometheusLabelSettings()
_METRIC_LABELS: Final[dict[str, list[str] | tuple[str, ...]]] = {
    "litellm_llm_api_latency_metric": _LLM_API_LATENCY_LABELS,
    "litellm_llm_api_time_to_first_token_metric": _LLM_API_LATENCY_LABELS,
    "litellm_request_total_latency_metric": _REQUEST_TOTAL_LATENCY_LABELS,
    "litellm_request_queue_time_seconds": _QUEUE_TIME, "litellm_guardrail_latency_seconds": _EMPTY,
    "litellm_guardrail_errors_total": _EMPTY, "litellm_guardrail_requests_total": _EMPTY,
    "litellm_proxy_total_requests_metric": _PROXY_TOTAL,
    "litellm_proxy_failed_requests_metric": _PROXY_FAILED_LABELS,
    "litellm_deployment_latency_per_output_token": _OUTPUT_TOKEN_LATENCY_LABELS,
    "litellm_overhead_latency_metric": _DEPLOYMENT_LIMIT_LABELS,
    "litellm_overhead_with_guardrails_latency_metric": _DEPLOYMENT_LIMIT_LABELS,
    "litellm_remaining_requests_metric": _DEPLOYMENT_LIMIT_LABELS,
    "litellm_remaining_tokens_metric": _DEPLOYMENT_LIMIT_LABELS,
    "litellm_requests_metric": _REQUESTS,
    "litellm_spend_metric": _SPEND, "litellm_zero_cost_requests_total": _ZERO_COST,
    "litellm_spend_capture_rate": _SPEND_CAPTURE_RATE_LABELS,
    "litellm_input_tokens_metric": _TOKENS,
    "litellm_total_tokens_metric": _TOKENS, "litellm_output_tokens_metric": _TOKENS,
    "litellm_input_cached_tokens_metric": _TOKENS,
    "litellm_input_cache_creation_tokens_metric": _TOKENS,
    "litellm_input_audio_tokens_metric": _TOKENS, "litellm_output_reasoning_tokens_metric": _TOKENS,
    "litellm_output_audio_tokens_metric": _TOKENS, "litellm_video_duration_seconds_metric": _TOKENS,
    "litellm_images_generated_metric": _TOKENS,
    "litellm_deployment_state": _DEPLOYMENT_STATE_LABELS,
    "litellm_deployment_tpm_limit": _DEPLOYMENT_RATE_LIMIT_LABELS,
    "litellm_deployment_rpm_limit": _DEPLOYMENT_RATE_LIMIT_LABELS,
    "litellm_deployment_cooled_down": _COOLDOWN,
    "litellm_deployment_successful_fallbacks": _FALLBACK,
    "litellm_deployment_failed_fallbacks": _FALLBACK,
    "litellm_remaining_team_budget_metric": _TEAM_BUDGET,
    "litellm_team_max_budget_metric": _TEAM_BUDGET,
    "litellm_team_budget_remaining_hours_metric": _TEAM_BUDGET,
    "litellm_team_members_metric": _TEAM_BUDGET, "litellm_remaining_org_budget_metric": _ORG_BUDGET,
    "litellm_org_max_budget_metric": _ORG_BUDGET,
    "litellm_org_budget_remaining_hours_metric": _ORG_BUDGET,
    "litellm_remaining_api_key_budget_metric": _KEY_BUDGET,
    "litellm_api_key_max_budget_metric": _KEY_BUDGET,
    "litellm_api_key_budget_remaining_hours_metric": _KEY_BUDGET,
    "litellm_remaining_user_budget_metric": _USER_BUDGET,
    "litellm_user_max_budget_metric": _USER_BUDGET,
    "litellm_user_budget_remaining_hours_metric": _USER_BUDGET,
    "litellm_remaining_customer_budget_metric": _CUSTOMER_BUDGET_LABELS,
    "litellm_customer_max_budget_metric": _CUSTOMER_BUDGET_LABELS,
    "litellm_customer_budget_remaining_hours_metric": _CUSTOMER_BUDGET_LABELS,
    "litellm_remaining_api_key_requests_for_model": _MODEL_LIMITS,
    "litellm_remaining_api_key_tokens_for_model": _MODEL_LIMITS,
    "litellm_callback_logging_failures_metric": _CALLBACK,
    "litellm_deployment_failure_responses": _FAIL_RESPONSES,
    "litellm_deployment_total_requests": _DEPLOYMENT_TOTAL_LABELS,
    "litellm_deployment_success_responses": _DEPLOYMENT_TOTAL_LABELS,
    "litellm_api_key_rate_limit_allowed_metric": _KEY_RATE,
    "litellm_api_key_rate_limit_used_metric": _KEY_RATE,
    "litellm_team_rate_limit_allowed_metric": _TEAM_RATE,
    "litellm_team_rate_limit_used_metric": _TEAM_RATE,
    "litellm_llm_api_failed_requests_metric": _FAILED_API, "litellm_pod_lock_manager_size": _EMPTY,
    "litellm_in_memory_daily_spend_update_queue_size": _EMPTY,
    "litellm_redis_daily_spend_update_queue_size": _EMPTY,
    "litellm_in_memory_spend_update_queue_size": _EMPTY,
    "litellm_redis_spend_update_queue_size": _EMPTY, "litellm_cache_hits_metric": _CACHE,
    "litellm_cache_misses_metric": _CACHE, "litellm_cached_tokens_metric": _CACHE,
    "litellm_provider_cache_read_input_tokens_metric": _CACHE,
    "litellm_provider_cache_creation_input_tokens_metric": _CACHE,
    "litellm_managed_batch_created_total": _BATCH, "litellm_managed_file_size_bytes": _EMPTY,
    "litellm_managed_batch_duration_seconds": _BATCH_TIME,
    "litellm_managed_file_created_total": _BATCH, "litellm_managed_file_deleted_total": _EMPTY,
    "litellm_check_batch_cost_jobs_polled": _EMPTY,
    "litellm_check_batch_cost_jobs_processed_total": _BATCH_TIME,
    "litellm_check_batch_cost_errors_total": _EMPTY,
    "litellm_check_batch_cost_last_run_timestamp": _EMPTY,
    "litellm_mcp_tool_calls_total": _MCP_TOOL_CALL_LABELS,
    "litellm_mcp_tool_call_spend_metric": _MCP_TOOL_CALL_LABELS,
}


class PrometheusMetricLabels:
    litellm_llm_api_latency_metric = _LLM_API_LATENCY_LABELS
    litellm_llm_api_time_to_first_token_metric = (  # ast-grep-ignore: no-long-names
        _LLM_API_LATENCY_LABELS)
    litellm_request_total_latency_metric = (  # ast-grep-ignore: no-long-names
        _REQUEST_TOTAL_LATENCY_LABELS)
    litellm_request_queue_time_seconds = _QUEUE_TIME  # ast-grep-ignore: no-long-names
    litellm_guardrail_latency_seconds = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_guardrail_errors_total = _EMPTY
    litellm_guardrail_requests_total = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_proxy_total_requests_metric = _PROXY_TOTAL  # ast-grep-ignore: no-long-names
    litellm_proxy_failed_requests_metric = _PROXY_FAILED_LABELS  # ast-grep-ignore: no-long-names
    litellm_deployment_latency_per_output_token = (  # ast-grep-ignore: no-long-names
        _OUTPUT_TOKEN_LATENCY_LABELS)
    litellm_overhead_latency_metric = _DEPLOYMENT_LIMIT_LABELS  # ast-grep-ignore: no-long-names
    litellm_overhead_with_guardrails_latency_metric = (  # ast-grep-ignore: no-long-names
        _DEPLOYMENT_LIMIT_LABELS)
    litellm_remaining_requests_metric = _DEPLOYMENT_LIMIT_LABELS  # ast-grep-ignore: no-long-names
    litellm_remaining_tokens_metric = _DEPLOYMENT_LIMIT_LABELS  # ast-grep-ignore: no-long-names
    litellm_requests_metric = _REQUESTS
    litellm_spend_metric = _SPEND
    litellm_zero_cost_requests_total = _ZERO_COST  # ast-grep-ignore: no-long-names
    litellm_spend_capture_rate = _SPEND_CAPTURE_RATE_LABELS
    litellm_input_tokens_metric = _TOKENS
    litellm_total_tokens_metric = _TOKENS
    litellm_output_tokens_metric = _TOKENS
    litellm_input_cached_tokens_metric = _TOKENS  # ast-grep-ignore: no-long-names
    litellm_input_cache_creation_tokens_metric = _TOKENS  # ast-grep-ignore: no-long-names
    litellm_input_audio_tokens_metric = _TOKENS  # ast-grep-ignore: no-long-names
    litellm_output_reasoning_tokens_metric = _TOKENS  # ast-grep-ignore: no-long-names
    litellm_output_audio_tokens_metric = _TOKENS  # ast-grep-ignore: no-long-names
    litellm_video_duration_seconds_metric = _TOKENS  # ast-grep-ignore: no-long-names
    litellm_images_generated_metric = _TOKENS  # ast-grep-ignore: no-long-names
    litellm_deployment_state = _DEPLOYMENT_STATE_LABELS
    litellm_deployment_tpm_limit = _DEPLOYMENT_RATE_LIMIT_LABELS
    litellm_deployment_rpm_limit = _DEPLOYMENT_RATE_LIMIT_LABELS
    litellm_deployment_cooled_down = _COOLDOWN
    litellm_deployment_successful_fallbacks = _FALLBACK  # ast-grep-ignore: no-long-names
    litellm_deployment_failed_fallbacks = _FALLBACK  # ast-grep-ignore: no-long-names
    litellm_remaining_team_budget_metric = _TEAM_BUDGET  # ast-grep-ignore: no-long-names
    litellm_team_max_budget_metric = _TEAM_BUDGET
    litellm_team_budget_remaining_hours_metric = _TEAM_BUDGET  # ast-grep-ignore: no-long-names
    litellm_team_members_metric = _TEAM_BUDGET
    litellm_remaining_org_budget_metric = _ORG_BUDGET  # ast-grep-ignore: no-long-names
    litellm_org_max_budget_metric = _ORG_BUDGET
    litellm_org_budget_remaining_hours_metric = _ORG_BUDGET  # ast-grep-ignore: no-long-names
    litellm_remaining_api_key_budget_metric = _KEY_BUDGET  # ast-grep-ignore: no-long-names
    litellm_api_key_max_budget_metric = _KEY_BUDGET  # ast-grep-ignore: no-long-names
    litellm_api_key_budget_remaining_hours_metric = _KEY_BUDGET  # ast-grep-ignore: no-long-names
    litellm_remaining_user_budget_metric = _USER_BUDGET  # ast-grep-ignore: no-long-names
    litellm_user_max_budget_metric = _USER_BUDGET
    litellm_user_budget_remaining_hours_metric = _USER_BUDGET  # ast-grep-ignore: no-long-names
    litellm_remaining_customer_budget_metric = (  # ast-grep-ignore: no-long-names
        _CUSTOMER_BUDGET_LABELS)
    litellm_customer_max_budget_metric = _CUSTOMER_BUDGET_LABELS  # ast-grep-ignore: no-long-names
    litellm_customer_budget_remaining_hours_metric = (  # ast-grep-ignore: no-long-names
        _CUSTOMER_BUDGET_LABELS)
    litellm_remaining_api_key_requests_for_model = _MODEL_LIMITS  # ast-grep-ignore: no-long-names
    litellm_remaining_api_key_tokens_for_model = _MODEL_LIMITS  # ast-grep-ignore: no-long-names
    litellm_callback_logging_failures_metric = _CALLBACK  # ast-grep-ignore: no-long-names
    litellm_deployment_failure_responses = _FAIL_RESPONSES  # ast-grep-ignore: no-long-names
    litellm_deployment_total_requests = _DEPLOYMENT_TOTAL_LABELS  # ast-grep-ignore: no-long-names
    litellm_deployment_success_responses = (  # ast-grep-ignore: no-long-names
        _DEPLOYMENT_TOTAL_LABELS)
    litellm_api_key_rate_limit_allowed_metric = _KEY_RATE  # ast-grep-ignore: no-long-names
    litellm_api_key_rate_limit_used_metric = _KEY_RATE  # ast-grep-ignore: no-long-names
    litellm_team_rate_limit_allowed_metric = _TEAM_RATE  # ast-grep-ignore: no-long-names
    litellm_team_rate_limit_used_metric = _TEAM_RATE  # ast-grep-ignore: no-long-names
    litellm_llm_api_failed_requests_metric = _FAILED_API  # ast-grep-ignore: no-long-names
    litellm_pod_lock_manager_size = _EMPTY
    litellm_in_memory_daily_spend_update_queue_size = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_redis_daily_spend_update_queue_size = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_in_memory_spend_update_queue_size = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_redis_spend_update_queue_size = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_cache_hits_metric = _CACHE
    litellm_cache_misses_metric = _CACHE
    litellm_cached_tokens_metric = _CACHE
    litellm_provider_cache_read_input_tokens_metric = _CACHE  # ast-grep-ignore: no-long-names
    litellm_provider_cache_creation_input_tokens_metric = _CACHE  # ast-grep-ignore: no-long-names
    litellm_managed_batch_created_total = _BATCH  # ast-grep-ignore: no-long-names
    litellm_managed_file_size_bytes = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_managed_batch_duration_seconds = _BATCH_TIME  # ast-grep-ignore: no-long-names
    litellm_managed_file_created_total = _BATCH  # ast-grep-ignore: no-long-names
    litellm_managed_file_deleted_total = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_check_batch_cost_jobs_polled = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_check_batch_cost_jobs_processed_total = _BATCH_TIME  # ast-grep-ignore: no-long-names
    litellm_check_batch_cost_errors_total = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_check_batch_cost_last_run_timestamp = _EMPTY  # ast-grep-ignore: no-long-names
    litellm_mcp_tool_calls_total = _MCP_TOOL_CALL_LABELS
    litellm_mcp_tool_call_spend_metric = _MCP_TOOL_CALL_LABELS  # ast-grep-ignore: no-long-names
    _cache_metric_labels = _CACHE
    _batch_user_labels = _BATCH
    _org_label_metrics: ClassVar[frozenset[str]] = _ORG_LABEL_METRICS
    _input_sequence_length_metrics: ClassVar[frozenset[str]] = _INPUT_SEQUENCE_LENGTH_METRICS

    @staticmethod
    def get_labels(label_name: DEFINED_PROMETHEUS_METRICS,
                   settings: PrometheusLabelSettings = _DEFAULT_SETTINGS) -> list[str]:
        if label_name not in _METRIC_LABELS:
            raise UnknownMetricError(label_name)
        default_labels = _resolve_deployment_and_latency_caller_identity_labels(
            metric_name=label_name, labels=_METRIC_LABELS[label_name])
        custom_labels = [_sanitize_prometheus_label_name(label)
                         for label in settings.custom_prometheus_metadata_labels]
        custom_labels.extend(_sanitize_prometheus_label_name(f"tag_{tag}")
                             for tag in settings.custom_prometheus_tags)
        for label in _opt_in_labels(label_name, settings):
            if label not in default_labels and label not in custom_labels:
                custom_labels.append(label)
        return [*default_labels, *custom_labels]


class UnknownMetricError(ValueError):
    def __init__(self, metric_name: str) -> None:
        super().__init__(f"unknown prometheus metric: {metric_name}")
        self.metric_name = metric_name


def _opt_in_labels(label_name: str, settings: PrometheusLabelSettings) -> list[str]:
    stream = settings.prometheus_emit_stream_label
    rate_limit = settings.prometheus_emit_rate_limit_labels
    email_alias = settings.prometheus_user_budget_label_include_email_alias
    sequence = settings.prometheus_emit_input_sequence_length_label
    groups = (
        (label_name == _PROXY_TOTAL_REQUESTS and stream, (UserAPIKeyLabelNames.STREAM.value,)),
        (label_name == _PROXY_FAILED_REQUESTS and rate_limit,
            (UserAPIKeyLabelNames.RATE_LIMIT_CATEGORY.value,
             UserAPIKeyLabelNames.RATE_LIMIT_TYPE.value)),
        (label_name in _USER_BUDGET_METRICS and email_alias,
            (UserAPIKeyLabelNames.USER_EMAIL.value, UserAPIKeyLabelNames.USER_ALIAS.value)),
        (label_name in _ORG_LABEL_METRICS,
            (UserAPIKeyLabelNames.ORG_ID.value, UserAPIKeyLabelNames.ORG_ALIAS.value)),
        (label_name in _INPUT_SEQUENCE_LENGTH_METRICS and sequence,
            (UserAPIKeyLabelNames.INPUT_SEQUENCE_LENGTH.value,)),
    )
    return [label for enabled, labels in groups if enabled for label in labels]


__all__ = ["PrometheusMetricLabels", "UnknownMetricError"]
