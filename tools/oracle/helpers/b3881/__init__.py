import asyncio
from datetime import UTC, datetime
from decimal import Decimal
from typing import Literal, NotRequired, TypedDict

from litellm.integrations.prometheus import PrometheusLogger
from litellm.types.integrations.prometheus import UserAPIKeyLabelValues
from pydantic import JsonValue


class Input(TypedDict):
    function: Literal["failure", "fallback_success", "fallback_failure", "success"]
    kwargs: NotRequired[dict[str, JsonValue]]
    payload: NotRequired[dict[str, JsonValue] | None]
    exception: NotRequired[dict[str, str] | None]
    values: NotRequired[dict[str, str]]
    original_model_group: NotRequired[str]
    start_time: NotRequired[str]
    completion_start_time: NotRequired[str | None]
    end_time: NotRequired[str]
    output_tokens: NotRequired[str | None]


class Recorder:
    def __init__(self, metric: str, events: list[JsonValue]) -> None:
        self._metric = metric
        self._events = events
        self._labels: dict[str, str] = {}

    def labels(self, **values: JsonValue) -> "Recorder":
        child = Recorder(self._metric, self._events)
        child._labels = {name: str(value) for name, value in values.items()}
        return child

    def inc(self, amount: float = 1.0) -> None:
        self._record("inc", amount)

    def set(self, amount: float) -> None:
        self._record("set", amount)

    def observe(self, amount: float) -> None:
        self._record("observe", amount)

    def _record(self, kind: str, amount: float) -> None:
        labels: dict[str, JsonValue] = dict(sorted(self._labels.items()))
        entry: JsonValue = {
            "metric": self._metric,
            "labels": labels,
            "kind": kind,
            "value": format(Decimal(str(amount)).normalize(), "f"),
        }
        self._events.append(entry)


class ProbeException(Exception):
    status_code: str | None = None
    code: str | None = None
    llm_provider: str | None = None

    def __init__(self, fields: dict[str, str] | None) -> None:
        super().__init__("probe")
        if fields is None:
            return
        status_code = fields.get("status_code")
        if status_code is not None:
            self.status_code = status_code
        code = fields.get("code")
        if code is not None:
            self.code = code
        llm_provider = fields.get("llm_provider")
        if llm_provider is not None:
            self.llm_provider = llm_provider


class ProbeLogger(PrometheusLogger):
    def __init__(self, events: list[JsonValue]) -> None:
        self._cached_metric_labels: dict[str, list[str]] = {}
        self.exclude_labels: frozenset[str] = frozenset()
        self.label_filters: dict[str, list[str]] = {}
        self.litellm_deployment_state = Recorder("litellm_deployment_state", events)
        self.litellm_deployment_failure_responses = Recorder("litellm_deployment_failure_responses", events)
        self.litellm_deployment_total_requests = Recorder("litellm_deployment_total_requests", events)
        self.litellm_deployment_success_responses = Recorder("litellm_deployment_success_responses", events)
        self.litellm_deployment_successful_fallbacks = Recorder("litellm_deployment_successful_fallbacks", events)
        self.litellm_deployment_failed_fallbacks = Recorder("litellm_deployment_failed_fallbacks", events)
        self.litellm_deployment_tpm_limit = Recorder("litellm_deployment_tpm_limit", events)
        self.litellm_deployment_rpm_limit = Recorder("litellm_deployment_rpm_limit", events)
        self.litellm_deployment_latency_per_output_token = Recorder(
            "litellm_deployment_latency_per_output_token", events
        )
        self.litellm_overhead_latency_metric = Recorder("litellm_overhead_latency_metric", events)
        self.litellm_overhead_with_guardrails_latency_metric = Recorder(
            "litellm_overhead_with_guardrails_latency_metric", events
        )
        self.litellm_remaining_requests_metric = Recorder("litellm_remaining_requests_metric", events)
        self.litellm_remaining_tokens_metric = Recorder("litellm_remaining_tokens_metric", events)


def labels_of(input_value: Input) -> UserAPIKeyLabelValues:
    return UserAPIKeyLabelValues(**input_value.get("values", {}))


def run(input_value: Input) -> JsonValue:
    events: list[JsonValue] = []
    logger = ProbeLogger(events)
    kwargs: dict[str, object] = {name: value for name, value in input_value.get("kwargs", {}).items()}
    payload = input_value.get("payload")
    if payload is not None:
        kwargs["standard_logging_object"] = payload
    exception = input_value.get("exception")
    if exception is not None:
        kwargs["exception"] = ProbeException(exception)
    match input_value["function"]:
        case "failure":
            logger.set_llm_deployment_failure_metrics(kwargs)
        case "fallback_success":
            asyncio.run(
                logger.log_success_fallback_event(
                    input_value.get("original_model_group", ""), kwargs, ProbeException(exception)
                )
            )
        case "fallback_failure":
            asyncio.run(
                logger.log_failure_fallback_event(
                    input_value.get("original_model_group", ""), kwargs, ProbeException(exception)
                )
            )
        case "success":
            completion = input_value.get("completion_start_time")
            if completion is not None:
                kwargs["completion_start_time"] = datetime.fromtimestamp(float(completion), tz=UTC)
            output_tokens = input_value.get("output_tokens")
            logger.set_llm_deployment_success_metrics(
                kwargs,
                datetime.fromtimestamp(float(input_value["start_time"]), tz=UTC),
                datetime.fromtimestamp(float(input_value["end_time"]), tz=UTC),
                labels_of(input_value),
                output_tokens=1.0 if output_tokens is None else float(output_tokens),
            )
    return {"events": events}
