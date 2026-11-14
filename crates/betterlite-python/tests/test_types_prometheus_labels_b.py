import inspect
from collections.abc import Sequence
from dataclasses import replace
from typing import Final, get_args

import litellm
import litellm.integrations.prometheus as litellm_prometheus
import pytest

from betterlite.types.integrations.prometheus.core import DEFINED_PROMETHEUS_METRICS
from betterlite.types.integrations.prometheus.metric_labels import (
    PrometheusMetricLabels,
    UnknownMetricError,
)
from betterlite.types.integrations.prometheus.settings import (
    NoOpMetric,
    PrometheusLabelSettings,
    PrometheusMetricsConfig,
    PrometheusSettings,
)
from betterlite.types.integrations.prometheus.validation import (
    validate_caller_identity_settings,
)
from betterlite.types.integrations.prometheus.values import UserAPIKeyLabelValues

MODE_NAMES: Final[tuple[str, ...]] = ("api_key_alias", "user_email", "both")
STREAM_LABEL_DUPLICATE_COUNT: Final = 2
ALL_METRIC_NAMES: Final[tuple[DEFINED_PROMETHEUS_METRICS, ...]] = get_args(
    DEFINED_PROMETHEUS_METRICS,
)
OUR_LABELS: Final[tuple[Sequence[str], ...]] = (
    PrometheusMetricLabels.litellm_llm_api_latency_metric,
    PrometheusMetricLabels.litellm_llm_api_time_to_first_token_metric,
    PrometheusMetricLabels.litellm_request_total_latency_metric,
    PrometheusMetricLabels.litellm_request_queue_time_seconds,
    PrometheusMetricLabels.litellm_guardrail_latency_seconds,
    PrometheusMetricLabels.litellm_guardrail_errors_total,
    PrometheusMetricLabels.litellm_guardrail_requests_total,
    PrometheusMetricLabels.litellm_proxy_total_requests_metric,
    PrometheusMetricLabels.litellm_proxy_failed_requests_metric,
    PrometheusMetricLabels.litellm_deployment_latency_per_output_token,
    PrometheusMetricLabels.litellm_overhead_latency_metric,
    PrometheusMetricLabels.litellm_overhead_with_guardrails_latency_metric,
    PrometheusMetricLabels.litellm_remaining_requests_metric,
    PrometheusMetricLabels.litellm_remaining_tokens_metric,
    PrometheusMetricLabels.litellm_requests_metric,
    PrometheusMetricLabels.litellm_spend_metric,
    PrometheusMetricLabels.litellm_zero_cost_requests_total,
    PrometheusMetricLabels.litellm_spend_capture_rate,
    PrometheusMetricLabels.litellm_input_tokens_metric,
    PrometheusMetricLabels.litellm_total_tokens_metric,
    PrometheusMetricLabels.litellm_output_tokens_metric,
    PrometheusMetricLabels.litellm_input_cached_tokens_metric,
    PrometheusMetricLabels.litellm_input_cache_creation_tokens_metric,
    PrometheusMetricLabels.litellm_input_audio_tokens_metric,
    PrometheusMetricLabels.litellm_output_reasoning_tokens_metric,
    PrometheusMetricLabels.litellm_output_audio_tokens_metric,
    PrometheusMetricLabels.litellm_video_duration_seconds_metric,
    PrometheusMetricLabels.litellm_images_generated_metric,
    PrometheusMetricLabels.litellm_deployment_state,
    PrometheusMetricLabels.litellm_deployment_tpm_limit,
    PrometheusMetricLabels.litellm_deployment_rpm_limit,
    PrometheusMetricLabels.litellm_deployment_cooled_down,
    PrometheusMetricLabels.litellm_deployment_successful_fallbacks,
    PrometheusMetricLabels.litellm_deployment_failed_fallbacks,
    PrometheusMetricLabels.litellm_remaining_team_budget_metric,
    PrometheusMetricLabels.litellm_team_max_budget_metric,
    PrometheusMetricLabels.litellm_team_budget_remaining_hours_metric,
    PrometheusMetricLabels.litellm_team_members_metric,
    PrometheusMetricLabels.litellm_remaining_org_budget_metric,
    PrometheusMetricLabels.litellm_org_max_budget_metric,
    PrometheusMetricLabels.litellm_org_budget_remaining_hours_metric,
    PrometheusMetricLabels.litellm_remaining_api_key_budget_metric,
    PrometheusMetricLabels.litellm_api_key_max_budget_metric,
    PrometheusMetricLabels.litellm_api_key_budget_remaining_hours_metric,
    PrometheusMetricLabels.litellm_remaining_user_budget_metric,
    PrometheusMetricLabels.litellm_user_max_budget_metric,
    PrometheusMetricLabels.litellm_user_budget_remaining_hours_metric,
    PrometheusMetricLabels.litellm_remaining_customer_budget_metric,
    PrometheusMetricLabels.litellm_customer_max_budget_metric,
    PrometheusMetricLabels.litellm_customer_budget_remaining_hours_metric,
    PrometheusMetricLabels.litellm_remaining_api_key_requests_for_model,
    PrometheusMetricLabels.litellm_remaining_api_key_tokens_for_model,
    PrometheusMetricLabels.litellm_callback_logging_failures_metric,
    PrometheusMetricLabels.litellm_deployment_failure_responses,
    PrometheusMetricLabels.litellm_deployment_total_requests,
    PrometheusMetricLabels.litellm_deployment_success_responses,
    PrometheusMetricLabels.litellm_api_key_rate_limit_allowed_metric,
    PrometheusMetricLabels.litellm_api_key_rate_limit_used_metric,
    PrometheusMetricLabels.litellm_team_rate_limit_allowed_metric,
    PrometheusMetricLabels.litellm_team_rate_limit_used_metric,
    PrometheusMetricLabels.litellm_llm_api_failed_requests_metric,
    PrometheusMetricLabels.litellm_pod_lock_manager_size,
    PrometheusMetricLabels.litellm_in_memory_daily_spend_update_queue_size,
    PrometheusMetricLabels.litellm_redis_daily_spend_update_queue_size,
    PrometheusMetricLabels.litellm_in_memory_spend_update_queue_size,
    PrometheusMetricLabels.litellm_redis_spend_update_queue_size,
    PrometheusMetricLabels.litellm_cache_hits_metric,
    PrometheusMetricLabels.litellm_cache_misses_metric,
    PrometheusMetricLabels.litellm_cached_tokens_metric,
    PrometheusMetricLabels.litellm_provider_cache_read_input_tokens_metric,
    PrometheusMetricLabels.litellm_provider_cache_creation_input_tokens_metric,
    PrometheusMetricLabels.litellm_managed_batch_created_total,
    PrometheusMetricLabels.litellm_managed_file_size_bytes,
    PrometheusMetricLabels.litellm_managed_batch_duration_seconds,
    PrometheusMetricLabels.litellm_managed_file_created_total,
    PrometheusMetricLabels.litellm_managed_file_deleted_total,
    PrometheusMetricLabels.litellm_check_batch_cost_jobs_polled,
    PrometheusMetricLabels.litellm_check_batch_cost_jobs_processed_total,
    PrometheusMetricLabels.litellm_check_batch_cost_errors_total,
    PrometheusMetricLabels.litellm_check_batch_cost_last_run_timestamp,
    PrometheusMetricLabels.litellm_mcp_tool_calls_total,
    PrometheusMetricLabels.litellm_mcp_tool_call_spend_metric,
)
THEIR_LABELS: Final[tuple[Sequence[str], ...]] = (
    litellm_prometheus.PrometheusMetricLabels.litellm_llm_api_latency_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_llm_api_time_to_first_token_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_request_total_latency_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_request_queue_time_seconds,
    litellm_prometheus.PrometheusMetricLabels.litellm_guardrail_latency_seconds,
    litellm_prometheus.PrometheusMetricLabels.litellm_guardrail_errors_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_guardrail_requests_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_proxy_total_requests_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_proxy_failed_requests_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_latency_per_output_token,
    litellm_prometheus.PrometheusMetricLabels.litellm_overhead_latency_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_overhead_with_guardrails_latency_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_requests_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_requests_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_spend_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_zero_cost_requests_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_spend_capture_rate,
    litellm_prometheus.PrometheusMetricLabels.litellm_input_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_total_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_output_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_input_cached_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_input_cache_creation_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_input_audio_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_output_reasoning_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_output_audio_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_video_duration_seconds_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_images_generated_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_state,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_tpm_limit,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_rpm_limit,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_cooled_down,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_successful_fallbacks,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_failed_fallbacks,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_team_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_team_max_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_team_budget_remaining_hours_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_team_members_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_org_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_org_max_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_org_budget_remaining_hours_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_api_key_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_api_key_max_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_api_key_budget_remaining_hours_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_user_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_user_max_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_user_budget_remaining_hours_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_customer_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_customer_max_budget_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_customer_budget_remaining_hours_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_api_key_requests_for_model,
    litellm_prometheus.PrometheusMetricLabels.litellm_remaining_api_key_tokens_for_model,
    litellm_prometheus.PrometheusMetricLabels.litellm_callback_logging_failures_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_failure_responses,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_total_requests,
    litellm_prometheus.PrometheusMetricLabels.litellm_deployment_success_responses,
    litellm_prometheus.PrometheusMetricLabels.litellm_api_key_rate_limit_allowed_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_api_key_rate_limit_used_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_team_rate_limit_allowed_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_team_rate_limit_used_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_llm_api_failed_requests_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_pod_lock_manager_size,
    litellm_prometheus.PrometheusMetricLabels.litellm_in_memory_daily_spend_update_queue_size,
    litellm_prometheus.PrometheusMetricLabels.litellm_redis_daily_spend_update_queue_size,
    litellm_prometheus.PrometheusMetricLabels.litellm_in_memory_spend_update_queue_size,
    litellm_prometheus.PrometheusMetricLabels.litellm_redis_spend_update_queue_size,
    litellm_prometheus.PrometheusMetricLabels.litellm_cache_hits_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_cache_misses_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_cached_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_provider_cache_read_input_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_provider_cache_creation_input_tokens_metric,
    litellm_prometheus.PrometheusMetricLabels.litellm_managed_batch_created_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_managed_file_size_bytes,
    litellm_prometheus.PrometheusMetricLabels.litellm_managed_batch_duration_seconds,
    litellm_prometheus.PrometheusMetricLabels.litellm_managed_file_created_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_managed_file_deleted_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_check_batch_cost_jobs_polled,
    litellm_prometheus.PrometheusMetricLabels.litellm_check_batch_cost_jobs_processed_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_check_batch_cost_errors_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_check_batch_cost_last_run_timestamp,
    litellm_prometheus.PrometheusMetricLabels.litellm_mcp_tool_calls_total,
    litellm_prometheus.PrometheusMetricLabels.litellm_mcp_tool_call_spend_metric,
)


@pytest.fixture(autouse=True)
def _reset_names_prometheus_state() -> None:
    litellm.custom_prometheus_metadata_labels = []
    litellm.custom_prometheus_tags = []
    litellm.prometheus_emit_stream_label = False
    litellm.prometheus_emit_rate_limit_labels = False
    litellm.prometheus_emit_input_sequence_length_label = False
    litellm.prometheus_user_budget_label_include_email_alias = False
    validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": "api_key_alias"},
    )
    litellm_prometheus.validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": "api_key_alias"},
    )


def _assert_all_labels_match(settings: PrometheusLabelSettings | None) -> None:
    for name in ALL_METRIC_NAMES:
        if name == "litellm_in_flight_requests":
            continue
        ours = (
            PrometheusMetricLabels.get_labels(name)
            if settings is None
            else PrometheusMetricLabels.get_labels(name, settings)
        )
        theirs = litellm_prometheus.PrometheusMetricLabels.get_labels(name)
        assert ours == theirs, name


def _settings_from_litellm() -> PrometheusLabelSettings:
    return PrometheusLabelSettings(
        custom_prometheus_metadata_labels=litellm.custom_prometheus_metadata_labels,
        custom_prometheus_tags=litellm.custom_prometheus_tags,
        prometheus_emit_stream_label=litellm.prometheus_emit_stream_label,
        prometheus_emit_rate_limit_labels=litellm.prometheus_emit_rate_limit_labels,
        prometheus_emit_input_sequence_length_label=(
            litellm.prometheus_emit_input_sequence_length_label
        ),
        prometheus_user_budget_label_include_email_alias=(
            litellm.prometheus_user_budget_label_include_email_alias
        ),
    )


def test_names_label_lists() -> None:
    assert len(OUR_LABELS) == len(THEIR_LABELS)
    assert OUR_LABELS == THEIR_LABELS


def test_names_class_members() -> None:
    ours = set(dir(PrometheusMetricLabels))
    theirs = set(dir(litellm_prometheus.PrometheusMetricLabels))
    assert ours == theirs


def test_names_get_labels_defaults() -> None:
    _assert_all_labels_match(None)
    assert PrometheusMetricLabels.get_labels("litellm_spend_metric") == [
        *PrometheusMetricLabels.litellm_spend_metric,
        "org_id",
        "org_alias",
    ]


def test_names_get_labels_custom() -> None:
    litellm.custom_prometheus_metadata_labels = ["my label", "x.y", "team", "9lives"]
    litellm.custom_prometheus_tags = ["team-name", "a b"]
    _assert_all_labels_match(_settings_from_litellm())


def test_names_get_labels_opt_in() -> None:
    litellm.prometheus_emit_stream_label = True
    litellm.prometheus_emit_rate_limit_labels = True
    litellm.prometheus_emit_input_sequence_length_label = True
    litellm.prometheus_user_budget_label_include_email_alias = True
    _assert_all_labels_match(_settings_from_litellm())


def test_names_get_labels_collide() -> None:
    litellm.custom_prometheus_metadata_labels = ["org_id", "user_email", "rate_limit_category"]
    litellm.prometheus_emit_rate_limit_labels = True
    litellm.prometheus_emit_input_sequence_length_label = True
    litellm.prometheus_user_budget_label_include_email_alias = True
    _assert_all_labels_match(_settings_from_litellm())


@pytest.mark.parametrize("mode", MODE_NAMES)
def test_names_caller_identity(mode: str) -> None:
    validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": mode},
    )
    litellm_prometheus.validate_caller_identity_settings(
        {"prometheus_deployment_and_latency_caller_identity": mode},
    )
    _assert_all_labels_match(None)


def test_names_reject_unknown() -> None:
    with pytest.raises(
        UnknownMetricError,
        match="litellm_in_flight_requests",
    ) as our_error:
        PrometheusMetricLabels.get_labels("litellm_in_flight_requests")
    assert our_error.value.metric_name == "litellm_in_flight_requests"
    with pytest.raises(AttributeError, match="litellm_in_flight_requests"):
        litellm_prometheus.PrometheusMetricLabels.get_labels("litellm_in_flight_requests")


def test_names_stream_deduped() -> None:
    litellm.custom_prometheus_metadata_labels = ["stream"]
    litellm.prometheus_emit_stream_label = True
    theirs = litellm_prometheus.PrometheusMetricLabels.get_labels(
        "litellm_proxy_total_requests_metric",
    )
    ours = PrometheusMetricLabels.get_labels(
        "litellm_proxy_total_requests_metric",
        PrometheusLabelSettings(
            custom_prometheus_metadata_labels=["stream"],
            prometheus_emit_stream_label=True,
        ),
    )
    assert theirs.count("stream") == STREAM_LABEL_DUPLICATE_COUNT
    assert ours.count("stream") == 1


def test_names_value_defaults() -> None:
    ours = UserAPIKeyLabelValues()
    theirs = litellm_prometheus.UserAPIKeyLabelValues()
    assert dict(ours.model_dump()) == theirs.model_dump()
    assert list(ours.model_dump()) == list(theirs.model_dump())


def test_names_label_value_fields() -> None:
    tag_values: list[str] = ["a", "b"]
    metadata: dict[str, str] = {"k": "v"}
    ours = UserAPIKeyLabelValues(
        end_user="eu",
        user="u",
        user_email="e@x",
        user_alias="ua",
        hashed_api_key="h",
        api_key_alias="ka",
        team="t",
        team_alias="ta",
        model_group="mg",
        requested_model="rm",
        model="m",
        litellm_model_name="lm",
        tags=tag_values,
        custom_metadata_labels=metadata,
        model_id="id",
        api_base="base",
        api_provider="prov",
        exception_status="es",
        exception_class="ec",
        rate_limit_category="rlc",
        rate_limit_type="rlt",
        status_code="sc",
        fallback_model="fm",
        route="r",
        client_ip="ip",
        user_agent="ua2",
        stream="stream",
        org_id="oid",
        org_alias="oa",
        mcp_tool_name="mt",
        mcp_server_name="ms",
        service_tier="st",
        input_sequence_length="isl",
    )
    theirs = litellm_prometheus.UserAPIKeyLabelValues(
        end_user="eu",
        user="u",
        user_email="e@x",
        user_alias="ua",
        hashed_api_key="h",
        api_key_alias="ka",
        team="t",
        team_alias="ta",
        model_group="mg",
        requested_model="rm",
        model="m",
        litellm_model_name="lm",
        tags=tag_values,
        custom_metadata_labels=metadata,
        model_id="id",
        api_base="base",
        api_provider="prov",
        exception_status="es",
        exception_class="ec",
        rate_limit_category="rlc",
        rate_limit_type="rlt",
        status_code="sc",
        fallback_model="fm",
        route="r",
        client_ip="ip",
        user_agent="ua2",
        stream="stream",
        org_id="oid",
        org_alias="oa",
        mcp_tool_name="mt",
        mcp_server_name="ms",
        service_tier="st",
        input_sequence_length="isl",
    )
    assert dict(ours.model_dump()) == theirs.model_dump()
    assert ours.tags == ("a", "b")
    our_metadata_type = type(ours.custom_metadata_labels)
    their_metadata_type = type(theirs.custom_metadata_labels)
    assert our_metadata_type is their_metadata_type
    replaced_ours = replace(ours, tags=["c"], user="u2")
    replaced_theirs = replace(theirs, tags=["c"], user="u2")
    assert dict(replaced_ours.model_dump()) == replaced_theirs.model_dump()


def test_names_value_rejects_keys() -> None:
    signature = inspect.signature(UserAPIKeyLabelValues)
    kinds = {parameter.kind.name for parameter in signature.parameters.values()}
    assert kinds == {"KEYWORD_ONLY"}
    with pytest.raises(TypeError):
        _ = signature.bind("end_user")
    with pytest.raises(TypeError):
        _ = signature.bind(bogus="1")
    with pytest.raises(TypeError):
        _ = signature.bind(api_key_hash="hashed")


def test_names_metric_config() -> None:
    ours = PrometheusMetricsConfig(group="g", metrics=["a", "b"])
    theirs = litellm_prometheus.PrometheusMetricsConfig(group="g", metrics=["a", "b"])
    assert (ours.group, ours.metrics, ours.include_labels) == (
        theirs.group,
        theirs.metrics,
        theirs.include_labels,
    )
    ours_with_labels = PrometheusMetricsConfig(group="g", metrics=["a"], include_labels=["m"])
    theirs_with_labels = litellm_prometheus.PrometheusMetricsConfig(
        group="g",
        metrics=["a"],
        include_labels=["m"],
    )
    assert ours_with_labels.include_labels == theirs_with_labels.include_labels
    positional_ours = PrometheusMetricsConfig("g", ["a"])
    positional_theirs = litellm_prometheus.PrometheusMetricsConfig("g", ["a"])
    assert (positional_ours.group, positional_ours.metrics) == (
        positional_theirs.group,
        positional_theirs.metrics,
    )
    our_settings = PrometheusSettings()
    their_settings = litellm_prometheus.PrometheusSettings()
    assert our_settings.prometheus_metrics_config is None
    assert their_settings.prometheus_metrics_config is None


def test_names_label_settings() -> None:
    settings = PrometheusLabelSettings()
    assert list(settings.custom_prometheus_metadata_labels) == (
        litellm.custom_prometheus_metadata_labels
    )
    assert list(settings.custom_prometheus_tags) == litellm.custom_prometheus_tags
    assert settings.prometheus_emit_stream_label == litellm.prometheus_emit_stream_label
    assert settings.prometheus_emit_rate_limit_labels == (
        litellm.prometheus_emit_rate_limit_labels
    )
    assert settings.prometheus_emit_input_sequence_length_label == (
        litellm.prometheus_emit_input_sequence_length_label
    )
    assert settings.prometheus_user_budget_label_include_email_alias == (
        litellm.prometheus_user_budget_label_include_email_alias
    )


def test_names_noop_metric_calls() -> None:
    metric = NoOpMetric()
    assert metric.labels("model", team="t") is metric
    metric.inc()
    metric.inc(2)
    metric.inc(amount=3)
    metric.set(1)
    metric.observe(0.5)
    metric.observe(amount=0.5)
    NoOpMetric(1, 2, key="value")
