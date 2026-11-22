import asyncio
from datetime import UTC, datetime
from decimal import Decimal
from typing import Literal, NotRequired, Protocol, TypedDict

from litellm.integrations.prometheus import PrometheusLogger
from litellm.types.integrations.prometheus import UserAPIKeyLabelValues
from pydantic import JsonValue
from typing_extensions import override


class Kwargs(TypedDict):
    model: NotRequired[str]
    stream: NotRequired[bool]
    tags: NotRequired[list[str]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    metadata: NotRequired[dict[str, JsonValue]]
    litellm_metadata: NotRequired[dict[str, JsonValue]]
    standard_logging_object: NotRequired[dict[str, JsonValue]]
    completion_start_time: NotRequired[datetime]
    exception: NotRequired[Exception]


class Input(TypedDict):
    function: Literal["failure", "fallback_success", "fallback_failure", "success"]
    start_time: str
    end_time: str
    kwargs: NotRequired[Kwargs]
    payload: NotRequired[dict[str, JsonValue]]
    exception: NotRequired[dict[str, str]]
    values: NotRequired[dict[str, str]]
    original_model_group: NotRequired[str]
    completion_start_time: NotRequired[str]
    output_tokens: NotRequired[str]


class _Recorder(Protocol):
    def labels(self, **values: JsonValue) -> "_Recorder": ...
    def inc(self, amount: float = 1.0) -> None: ...
    def set(self, amount: float) -> None: ...
    def observe(self, amount: float) -> None: ...


class Recorder(_Recorder):
    def __init__(self, metric: str, events: list[JsonValue]) -> None:
        self._metric = metric
        self._events = events
        self._labels: dict[str, JsonValue] = {}

    @override
    def labels(self, **values: JsonValue) -> "_Recorder":
        child = Recorder(self._metric, self._events)
        child._labels = dict(values)
        return child

    @override
    def inc(self, amount: float = 1.0) -> None:
        self._record("inc", amount)

    @override
    def set(self, amount: float) -> None:
        self._record("set", amount)

    @override
    def observe(self, amount: float) -> None:
        self._record("observe", amount)

    def _record(self, kind: str, amount: float) -> None:
        labels: dict[str, JsonValue] = {name: str(value) for name, value in self._labels.items()}
        entry: JsonValue = {
            "metric": self._metric,
            "labels": dict(sorted(labels.items())),
            "kind": kind,
            "value": format(Decimal(str(amount)).normalize(), "f"),
        }
        self._events.append(entry)


class ProbeError(Exception):
    status_code: str | None = None
    code: str | None = None
    llm_provider: str | None = None

    def __init__(self, fields: dict[str, str] | None) -> None:
        super().__init__("probe")
        if fields is None:
            return
        self.status_code = fields.get("status_code")
        self.code = fields.get("code")
        self.llm_provider = fields.get("llm_provider")


class ProbeLogger(PrometheusLogger):
    def __init__(self, events: list[JsonValue]) -> None:
        self._cached_metric_labels: dict[str, list[str]] = {}
        self.exclude_labels: frozenset[str] = frozenset()
        self.label_filters: dict[str, list[str]] = {}
        self.litellm_deployment_state = Recorder("litellm_deployment_state", events)
        self.litellm_deployment_failure_responses = Recorder(
            "litellm_deployment_failure_responses", events
        )
        self.litellm_deployment_total_requests = Recorder(
            "litellm_deployment_total_requests", events
        )
        self.litellm_deployment_success_responses = Recorder(
            "litellm_deployment_success_responses", events
        )
        self.litellm_deployment_successful_fallbacks = Recorder(
            "litellm_deployment_successful_fallbacks", events
        )
        self.litellm_deployment_failed_fallbacks = Recorder(
            "litellm_deployment_failed_fallbacks", events
        )
        self.litellm_deployment_tpm_limit = Recorder("litellm_deployment_tpm_limit", events)
        self.litellm_deployment_rpm_limit = Recorder("litellm_deployment_rpm_limit", events)
        self.litellm_deployment_latency_per_output_token = Recorder(
            "litellm_deployment_latency_per_output_token", events
        )
        self.litellm_overhead_latency_metric = Recorder("litellm_overhead_latency_metric", events)
        self.litellm_overhead_with_guardrails_latency_metric = Recorder(
            "litellm_overhead_with_guardrails_latency_metric", events
        )
        self.litellm_remaining_requests_metric = Recorder(
            "litellm_remaining_requests_metric", events
        )
        self.litellm_remaining_tokens_metric = Recorder("litellm_remaining_tokens_metric", events)


async def _log_fallback(
    logger: ProbeLogger,
    function: Literal["fallback_success", "fallback_failure"],
    group: str,
    kwargs: Kwargs,
    error: ProbeError,
) -> None:
    if function == "fallback_success":
        await logger.log_success_fallback_event(group, kwargs, error)  # type: ignore[misc, arg-type]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3434
    else:
        await logger.log_failure_fallback_event(group, kwargs, error)  # type: ignore[misc, arg-type]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3466


def run(input_value: Input) -> JsonValue:
    events: list[JsonValue] = []
    logger = ProbeLogger(events)
    given = input_value.get("kwargs")
    kwargs: Kwargs = {} if given is None else given
    payload = input_value.get("payload")
    if payload is not None:
        kwargs["standard_logging_object"] = payload
    exception = input_value.get("exception")
    error = ProbeError(exception)
    if exception is not None:
        kwargs["exception"] = error
    function = input_value["function"]
    if function == "failure":
        logger.set_llm_deployment_failure_metrics(kwargs)  # type: ignore[arg-type]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L2794
    elif function == "success":
        values = input_value.get("values")
        enum_values = UserAPIKeyLabelValues() if values is None else UserAPIKeyLabelValues(**values)
        completion = input_value.get("completion_start_time")
        if completion is not None:
            kwargs["completion_start_time"] = datetime.fromtimestamp(float(completion), tz=UTC)
        tokens = input_value.get("output_tokens")
        logger.set_llm_deployment_success_metrics(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3066
            kwargs,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/prometheus.py#L3066
            datetime.fromtimestamp(float(input_value["start_time"]), tz=UTC),
            datetime.fromtimestamp(float(input_value["end_time"]), tz=UTC),
            enum_values,
            output_tokens=1.0 if tokens is None else float(tokens),
        )
    else:
        group = input_value.get("original_model_group")
        asyncio.run(_log_fallback(logger, function, "" if group is None else group, kwargs, error))
    return {"events": events}
