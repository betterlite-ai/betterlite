from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.integrations.opentelemetry import OpenTelemetry, OpenTelemetryConfig
from litellm.integrations.opentelemetry_utils.gen_ai_semconv import OTELSemconvCategory
from litellm.integrations.otel.model.request_io import request_input, response_output, stream_output
from litellm.types.llms.openai import ResponseCompletedEvent, ResponsesAPIResponse
from litellm.types.utils import ModelResponse, ModelResponseStream
from opentelemetry.sdk._logs import LoggerProvider
from opentelemetry.sdk._logs.export import InMemoryLogRecordExporter, SimpleLogRecordProcessor
from opentelemetry.sdk.metrics import MeterProvider
from opentelemetry.sdk.metrics.export import InMemoryMetricReader
from opentelemetry.sdk.trace import TracerProvider
from pydantic import JsonValue, TypeAdapter, ValidationError


class Config(TypedDict, total=False):
    enable_events: bool
    semconv: bool
    capture_mode: str
    attributes: JsonValue


class Chunk(TypedDict):
    type: Literal["chat", "completed", "other"]
    value: dict[str, JsonValue]


class Input(TypedDict):
    function: Literal["record_success", "request_input", "response_output", "stream_output"]
    config: NotRequired[Config]
    callback_name: NotRequired[str | None]
    callback_settings: NotRequired[JsonValue]
    kwargs: NotRequired[dict[str, JsonValue]]
    response: NotRequired[dict[str, JsonValue] | None]
    start_time: NotRequired[float]
    end_time: NotRequired[float]
    data: NotRequired[dict[str, JsonValue]]
    kind: NotRequired[Literal["model_response", "responses_api", "other"]]
    chunks: NotRequired[list[Chunk]]


class DataPoint(TypedDict):
    attributes: dict[str, str | int]
    count: int
    sum: float


class MetricData(TypedDict):
    data_points: list[DataPoint]


class Metric(TypedDict):
    name: str
    data: MetricData


class ScopeMetrics(TypedDict):
    metrics: list[Metric]


class ResourceMetrics(TypedDict):
    scope_metrics: list[ScopeMetrics]


class MetricsPayload(TypedDict):
    resource_metrics: list[ResourceMetrics]


class MetricLine(TypedDict):
    name: str
    attributes: dict[str, JsonValue]
    count: int
    sum: str


class EventLine(TypedDict):
    body: JsonValue
    attributes: dict[str, JsonValue]


class Recorded(TypedDict):
    metrics: list[MetricLine]
    events: list[EventLine]


INPUT = TypeAdapter(Input)
METRICS = TypeAdapter(MetricsPayload)
JSON = TypeAdapter(JsonValue)
TEXTS = TypeAdapter(dict[str, str | int])


def metric_key(line: MetricLine) -> str:
    token_type = line["attributes"].get("gen_ai.token.type")
    return f"{line['name']} {token_type}"


# the string attributes carry JSON text where a container was written, and both sides read it back
def attribute_value(value: str | int) -> JsonValue:
    match value:
        case int():
            return value
        case str():
            if not value.startswith(("{", "[")):
                return value
            try:
                return JSON.validate_json(value)
            except ValidationError:
                return value


def attribute_values(attributes: dict[str, str | int]) -> dict[str, JsonValue]:
    return {key: attribute_value(value) for key, value in attributes.items()}


def metric_lines(reader: InMemoryMetricReader) -> list[MetricLine]:
    data = reader.get_metrics_data()
    if data is None:
        return []
    payload = METRICS.validate_json(data.to_json())
    lines = [
        MetricLine(
            name=metric["name"],
            attributes=attribute_values(point["attributes"]),
            count=point["count"],
            sum=f"{point['sum']:.9f}",
        )
        for resource in payload["resource_metrics"]
        for scope in resource["scope_metrics"]
        for metric in scope["metrics"]
        for point in metric["data"]["data_points"]
    ]
    return sorted(lines, key=metric_key)


def new_logger(
    request: Input, reader: InMemoryMetricReader, logger_provider: LoggerProvider
) -> OpenTelemetry:
    settings = request["config"] if "config" in request else Config()
    categories: set[OTELSemconvCategory] = set()
    if settings.get("semconv") is True:
        categories.add(OTELSemconvCategory.GEN_AI_LATEST_EXPERIMENTAL)
    config = OpenTelemetryConfig(
        exporter="in_memory",
        enable_metrics=True,
        enable_events=settings.get("enable_events") is not False,
        skip_set_global=True,
        capture_message_content=settings.get("capture_mode"),
        semconv_stability_opt_in=categories,
    )
    extra = {"attributes": settings["attributes"]} if "attributes" in settings else {}
    return OpenTelemetry(
        config=config,
        callback_name=request.get("callback_name"),
        tracer_provider=TracerProvider(),
        meter_provider=MeterProvider(metric_readers=[reader]),
        logger_provider=logger_provider,
        **extra,
    )


def record_success(request: Input) -> Recorded:
    reader = InMemoryMetricReader()
    log_exporter = InMemoryLogRecordExporter()
    logger_provider = LoggerProvider()
    logger_provider.add_log_record_processor(SimpleLogRecordProcessor(log_exporter))
    previous = litellm.callback_settings
    litellm.callback_settings = {}
    if "callback_settings" in request:
        litellm.callback_settings = {"otel": {"attributes": request["callback_settings"]}}
    try:
        new_logger(request, reader, logger_provider).log_success_event(
            request["kwargs"],
            request.get("response"),
            datetime.fromtimestamp(request["start_time"], tz=UTC),
            datetime.fromtimestamp(request["end_time"], tz=UTC),
        )
    finally:
        litellm.callback_settings = previous
    events = [
        EventLine(
            body=JSON.validate_python(log.log_record.body),
            attributes=attribute_values(
                TEXTS.validate_python(dict(log.log_record.attributes or {}))
            ),
        )
        for log in log_exporter.get_finished_logs()
    ]
    return Recorded(metrics=metric_lines(reader), events=events)


def response_text(request: Input) -> str | None:
    response = request["response"]
    match request["kind"]:
        case "model_response":
            return response_output(ModelResponse.model_validate(response))
        case "responses_api":
            return response_output(ResponsesAPIResponse.model_validate(response))
        case "other":
            return response_output(response)


def parsed(text: str | None) -> JsonValue:
    if text is None:
        return None
    return JSON.validate_json(text)


Relayed = ModelResponseStream | ResponseCompletedEvent | dict[str, JsonValue]


def relayed(chunks: list[Chunk]) -> list[Relayed]:
    typed: list[Relayed] = []
    for chunk in chunks:
        match chunk["type"]:
            case "chat":
                typed.append(ModelResponseStream.model_validate(chunk["value"]))
            case "completed":
                typed.append(ResponseCompletedEvent.model_validate(chunk["value"]))
            case "other":
                typed.append(chunk["value"])
    return typed


def run(input_value: Input) -> JsonValue:
    request = INPUT.validate_python(input_value)
    match request["function"]:
        case "record_success":
            return TypeAdapter(Recorded).dump_python(record_success(request))
        case "request_input":
            return parsed(request_input(request["data"]))
        case "response_output":
            return parsed(response_text(request))
        case "stream_output":
            return parsed(stream_output(relayed(request["chunks"]), request["data"]))
