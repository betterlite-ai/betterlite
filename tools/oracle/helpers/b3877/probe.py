from typing import Protocol, cast

from litellm.integrations.prometheus import PrometheusLogger
from litellm.integrations.prometheus_helpers import PrometheusLabelFactoryContext
from litellm.types.integrations.prometheus import (
    DEFINED_PROMETHEUS_METRICS,
    UserAPIKeyLabelValues,
)
from litellm.types.utils import (
    StandardLoggingPayload,
    StandardLoggingZeroCostDiagnostic as ZeroCostDiagnostic,
)
from pydantic import JsonValue
from typing_extensions import override


class _Metric(Protocol):
    def labels(self, **values: JsonValue) -> "_Metric": ...
    def inc(self, amount: float = 1.0) -> None: ...
    def set(self, amount: float) -> None: ...


class _ProbeMetric(_Metric):
    def __init__(self, name: str, log: list[JsonValue]) -> None:
        self._name = name
        self._log = log
        self._labels: dict[str, JsonValue] = {}

    @override
    def labels(self, **values: JsonValue) -> "_ProbeMetric":
        child = _ProbeMetric(self._name, self._log)
        child._labels = dict(values)
        return child

    @override
    def inc(self, amount: float = 1.0) -> None:
        self._record("increment", amount)

    @override
    def set(self, amount: float) -> None:
        self._record("set", amount)

    def _record(self, event: str, amount: float) -> None:
        measured = float(amount)
        text = str(int(measured)) if measured.is_integer() else str(measured)
        entry: JsonValue = {
            "event": event,
            "metric": self._name,
            "labels": {name: str(value) for name, value in self._labels.items()},
            "amount": text,
        }
        self._log.append(entry)


class ProbeLogger(PrometheusLogger):
    def __init__(self, supported: dict[str, list[str]], log: list[JsonValue]) -> None:
        self._supported = supported
        self._log = log
        self.litellm_requests_metric = _ProbeMetric("litellm_requests_metric", log)
        self.litellm_spend_metric = _ProbeMetric("litellm_spend_metric", log)
        self.litellm_zero_cost_requests_total = _ProbeMetric(
            "litellm_zero_cost_requests_total", log
        )
        self.litellm_tokens_metric = _ProbeMetric("litellm_total_tokens_metric", log)
        self.litellm_input_tokens_metric = _ProbeMetric("litellm_input_tokens_metric", log)
        self.litellm_output_tokens_metric = _ProbeMetric("litellm_output_tokens_metric", log)
        self.litellm_input_cached_tokens_metric = _ProbeMetric(
            "litellm_input_cached_tokens_metric", log
        )
        self.litellm_input_cache_creation_tokens_metric = _ProbeMetric(
            "litellm_input_cache_creation_tokens_metric", log
        )
        self.litellm_input_audio_tokens_metric = _ProbeMetric(
            "litellm_input_audio_tokens_metric", log
        )
        self.litellm_output_reasoning_tokens_metric = _ProbeMetric(
            "litellm_output_reasoning_tokens_metric", log
        )
        self.litellm_output_audio_tokens_metric = _ProbeMetric(
            "litellm_output_audio_tokens_metric", log
        )
        self.litellm_video_duration_seconds_metric = _ProbeMetric(
            "litellm_video_duration_seconds_metric", log
        )
        self.litellm_images_generated_metric = _ProbeMetric("litellm_images_generated_metric", log)
        self.litellm_cache_hits_metric = _ProbeMetric("litellm_cache_hits_metric", log)
        self.litellm_cache_misses_metric = _ProbeMetric("litellm_cache_misses_metric", log)
        self.litellm_cached_tokens_metric = _ProbeMetric("litellm_cached_tokens_metric", log)
        self.litellm_provider_cache_read_input_tokens_metric = _ProbeMetric(
            "litellm_provider_cache_read_input_tokens_metric", log
        )
        self.litellm_provider_cache_creation_input_tokens_metric = _ProbeMetric(
            "litellm_provider_cache_creation_input_tokens_metric", log
        )
        self.litellm_mcp_tool_calls_total = _ProbeMetric("litellm_mcp_tool_calls_total", log)
        self.litellm_mcp_tool_call_spend_metric = _ProbeMetric(
            "litellm_mcp_tool_call_spend_metric", log
        )

    @override
    def get_labels_for_metric(self, metric_name: DEFINED_PROMETHEUS_METRICS) -> list[str]:
        labels = self._supported.get(metric_name)
        if labels is None:
            return []
        return labels

    def entries(self) -> list[JsonValue]:
        return self._log

    def request_spend(self, enum_values: UserAPIKeyLabelValues, response_cost: float) -> None:
        self._increment_top_level_request_and_spend_metrics(
            end_user_id=None, user_api_key=None, user_api_key_alias=None, model=None,
            user_api_team=None, user_api_team_alias=None, user_id=None,
            response_cost=response_cost, enum_values=enum_values,
        )

    def token_metrics(self, payload: JsonValue, enum_values: UserAPIKeyLabelValues) -> None:
        self._increment_token_metrics(
            standard_logging_payload=cast("StandardLoggingPayload", payload),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1553-L1611
            end_user_id=None, user_api_key=None, user_api_key_alias=None, model=None,
            user_api_team=None, user_api_team_alias=None, user_id=None,
            enum_values=enum_values,
        )

    def zero_cost(self, enum_values: UserAPIKeyLabelValues) -> None:
        diagnostic: ZeroCostDiagnostic = {
            "reason": "missing_pricing_key",
            "pricing_model": "gpt-4o",
            "missing_pricing_keys": (),
        }
        context = PrometheusLabelFactoryContext(enum_values)
        self._increment_zero_cost_requests_metric(diagnostic, enum_values, context)

    def token_detail(self, payload: JsonValue, enum_values: UserAPIKeyLabelValues) -> None:
        self._increment_token_detail_metrics(
            standard_logging_payload=cast("StandardLoggingPayload", payload),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1613-L1672
            enum_values=enum_values,
        )

    def media(self, payload: JsonValue, enum_values: UserAPIKeyLabelValues) -> None:
        self._increment_media_generation_metrics(
            standard_logging_payload=cast("StandardLoggingPayload", payload),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1674-L1710
            enum_values=enum_values,
        )

    def cache(self, payload: JsonValue, enum_values: UserAPIKeyLabelValues) -> None:
        self._increment_cache_metrics(
            standard_logging_payload=cast("StandardLoggingPayload", payload),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L1736-L1812
            enum_values=enum_values,
        )

    @classmethod
    def provider_cache(cls, usage: dict[str, JsonValue]) -> tuple[int, int]:
        return cls._resolve_provider_cache_tokens(usage)

    @classmethod
    def cache_write(cls, details: dict[str, JsonValue]) -> int | None:
        return cls._resolve_cache_write_tokens(details)
