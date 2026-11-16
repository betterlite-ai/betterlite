from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict, cast

import litellm
from litellm.integrations.opentelemetry import TOKEN_TYPE_ATTRIBUTE
from litellm.integrations.otel.plumbing.metrics import (
    GenAIMetricRecorder,
    create_genai_metrics,
)
from opentelemetry.sdk.metrics import MeterProvider
from opentelemetry.sdk.metrics.export import InMemoryMetricReader
from pydantic import JsonValue, TypeAdapter, ValidationError


class CallKwargs(TypedDict, total=False):
    model: str
    call_type: str | None
    litellm_params: dict[str, JsonValue] | None
    standard_logging_object: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    response_cost: float | None
    completion_start_time: float | str | None
    api_call_start_time: float | str | None
    end_time: float | str | None


class TokenUsage(TypedDict, total=False):
    prompt_tokens: int
    completion_tokens: int


class CallBase(TypedDict):
    kwargs: CallKwargs
    start_time: float
    end_time: float
    callback_name: NotRequired[str | None]
    callback_settings: NotRequired[JsonValue]


class RecordCall(CallBase):
    function: Literal["record"]
    response: NotRequired[dict[str, JsonValue] | None]


class RecordFailureCall(CallBase):
    function: Literal["record_failure"]


class RawPoint(TypedDict):
    attributes: dict[str, str]
    count: int
    sum: float


class RawHistogram(TypedDict):
    data_points: list[RawPoint]


class RawMetric(TypedDict):
    name: str
    data: RawHistogram


class RawScope(TypedDict):
    metrics: list[RawMetric]


class RawResource(TypedDict):
    scope_metrics: list[RawScope]


class RawPayload(TypedDict):
    resource_metrics: list[RawResource]


class Point(TypedDict):
    attributes: dict[str, JsonValue]
    count: int
    sum: str


class RecordedMetrics(TypedDict):
    metrics: dict[str, list[Point]]


JSON = TypeAdapter(JsonValue)
PAYLOAD = TypeAdapter(RawPayload)
USAGE = TypeAdapter(TokenUsage)


class ResponseView:
    def __init__(self, response: dict[str, JsonValue]) -> None:
        self._response = response

    def get(self, key: Literal["usage"], /) -> TokenUsage | None:
        usage = self._response.get(key)
        if usage is None:
            return None
        return USAGE.validate_python(usage)


def cell(text: str) -> JsonValue:
    if not text.startswith(("{", "[")):
        return text
    try:
        return JSON.validate_json(text)
    except ValidationError:
        return text


def token_type_of(point: RawPoint) -> str:
    attributes = point["attributes"]
    token_type = attributes.get(TOKEN_TYPE_ATTRIBUTE)
    if token_type is None:
        return ""
    return token_type


def points_of(metric: RawMetric) -> list[Point]:
    ordered = sorted(metric["data"]["data_points"], key=token_type_of)
    return [
        Point(
            attributes={key: cell(text) for key, text in point["attributes"].items()},
            count=point["count"],
            sum=f"{point['sum']:.9f}",
        )
        for point in ordered
    ]


def metric_points(reader: InMemoryMetricReader) -> dict[str, list[Point]]:
    data = reader.get_metrics_data()
    if data is None:
        return {}
    payload = PAYLOAD.validate_json(data.to_json())
    return {
        metric["name"]: points_of(metric)
        for resource in payload["resource_metrics"]
        for scope in resource["scope_metrics"]
        for metric in scope["metrics"]
    }


def response_view(response: dict[str, JsonValue] | None) -> ResponseView | None:
    if response is None:
        return None
    return ResponseView(response)


def record_metrics(request: RecordCall | RecordFailureCall) -> RecordedMetrics:
    reader = InMemoryMetricReader()
    provider = MeterProvider(metric_readers=[reader])
    recorder = GenAIMetricRecorder(
        create_genai_metrics(provider.get_meter("b3869")), request.get("callback_name")
    )
    shared = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L184
        "dict[str, dict[str, JsonValue]]", litellm.callback_settings
    )
    shared.clear()
    settings = request.get("callback_settings")
    if settings is not None:
        shared["otel"] = {"attributes": settings}
    start = datetime.fromtimestamp(request["start_time"], tz=UTC)
    end = datetime.fromtimestamp(request["end_time"], tz=UTC)
    if request["function"] == "record":
        recorder.record(request["kwargs"], response_view(request.get("response")), start, end)
    else:
        recorder.record_failure(request["kwargs"], start, end)
    return RecordedMetrics(metrics=metric_points(reader))


def run(input_value: RecordCall | RecordFailureCall) -> JsonValue:
    recorded = record_metrics(input_value)
    return JSON.validate_python(recorded)
