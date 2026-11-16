from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, TypedDict, cast

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


class RecordSuccessCall(TypedDict):
    function: Literal["record_success"]
    kwargs: dict[str, JsonValue]
    start_time: float
    end_time: float
    config: NotRequired[dict[str, JsonValue]]
    callback_name: NotRequired[str | None]
    callback_settings: NotRequired[JsonValue]
    response: NotRequired[dict[str, JsonValue] | None]


class RequestInputCall(TypedDict):
    function: Literal["request_input"]
    data: dict[str, JsonValue]


class ResponseOutputCall(TypedDict):
    function: Literal["response_output"]
    kind: Literal["model_response", "responses_api", "other"]
    response: dict[str, JsonValue]


class Chunk(TypedDict):
    type: Literal["chat", "completed", "other"]
    value: dict[str, JsonValue]


class StreamOutputCall(TypedDict):
    function: Literal["stream_output"]
    chunks: list[Chunk]
    data: dict[str, JsonValue]


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


METRICS = TypeAdapter(MetricsPayload)
JSON = TypeAdapter(JsonValue)
TEXTS = TypeAdapter(dict[str, str | int])
TEXT = TypeAdapter(str | None)
CHUNK_VALUE = TypeAdapter(dict[str, JsonValue])
MODEL_RESPONSE: Final[TypeAdapter[ModelResponse]] = TypeAdapter(ModelResponse)
RESPONSES_API: Final[TypeAdapter[ResponsesAPIResponse]] = TypeAdapter(ResponsesAPIResponse)
STREAM_CHUNK: Final[TypeAdapter[ModelResponseStream]] = TypeAdapter(ModelResponseStream)
COMPLETED_EVENT: Final[TypeAdapter[ResponseCompletedEvent]] = TypeAdapter(ResponseCompletedEvent)
# InMemoryLogRecordExporter.__init__ is unannotated; this typed reference keeps the call checked
LOG_EXPORTER: Final[type[InMemoryLogRecordExporter]] = InMemoryLogRecordExporter


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


def metric_lines(reader: InMemoryMetricReader) -> list[MetricLine]:
    data = reader.get_metrics_data()
    if data is None:
        return []
    payload = METRICS.validate_json(data.to_json())
    lines = [
        MetricLine(
            name=metric["name"],
            attributes={key: attribute_value(value) for key, value in point["attributes"].items()},
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
    request: RecordSuccessCall,
    reader: InMemoryMetricReader,
    logger_provider: LoggerProvider,
) -> OpenTelemetry:
    settings: dict[str, JsonValue] = {}
    if "config" in request:
        settings = request["config"]
    categories: set[OTELSemconvCategory] = set()
    if settings.get("semconv") is True:
        categories.add(OTELSemconvCategory.GEN_AI_LATEST_EXPERIMENTAL)
    config = OpenTelemetryConfig(
        exporter="in_memory",
        enable_metrics=True,
        enable_events=settings.get("enable_events") is not False,
        skip_set_global=True,
        capture_message_content=TEXT.validate_python(settings.get("capture_mode")),
        semconv_stability_opt_in=categories,
    )
    return OpenTelemetry(
        config=config,
        callback_name=request.get("callback_name"),
        tracer_provider=TracerProvider(),
        meter_provider=MeterProvider(metric_readers=[reader]),
        logger_provider=logger_provider,
        attributes=settings.get("attributes"),
    )


def record_success(request: RecordSuccessCall) -> Recorded:
    reader = InMemoryMetricReader()
    log_exporter = LOG_EXPORTER()
    logger_provider = LoggerProvider()
    logger_provider.add_log_record_processor(SimpleLogRecordProcessor(log_exporter))
    callback_settings = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L184
        "dict[str, dict[str, JsonValue]]", litellm.callback_settings
    )
    previous = callback_settings.copy()
    callback_settings.clear()
    if "callback_settings" in request:
        callback_settings["otel"] = {"attributes": request["callback_settings"]}
    try:
        logger = new_logger(request, reader, logger_provider)
        logger.log_success_event(  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L756
            request["kwargs"],
            request.get("response"),
            datetime.fromtimestamp(request["start_time"], tz=UTC),
            datetime.fromtimestamp(request["end_time"], tz=UTC),
        )
    finally:
        callback_settings.clear()
        callback_settings.update(previous)
    events: list[EventLine] = []
    for log in log_exporter.get_finished_logs():
        raw = log.log_record.attributes
        attributes: dict[str, str | int] = {}
        if raw is not None:
            texts = dict(raw)
            attributes = TEXTS.validate_python(texts)
        values = {key: attribute_value(value) for key, value in attributes.items()}
        body = JSON.validate_python(log.log_record.body)
        events.append(EventLine(body=body, attributes=values))
    return Recorded(metrics=metric_lines(reader), events=events)


def response_text(request: ResponseOutputCall) -> str | None:
    match request["kind"]:
        case "model_response":
            return response_output(MODEL_RESPONSE.validate_python(request["response"]))
        case "responses_api":
            return response_output(RESPONSES_API.validate_python(request["response"]))
        case "other":
            return response_output(request["response"])


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
                typed.append(STREAM_CHUNK.validate_python(chunk["value"]))
            case "completed":
                typed.append(COMPLETED_EVENT.validate_python(chunk["value"]))
            case "other":
                typed.append(CHUNK_VALUE.validate_python(chunk["value"]))
    return typed


def run(
    input_value: RecordSuccessCall | RequestInputCall | ResponseOutputCall | StreamOutputCall,
) -> JsonValue:
    if input_value["function"] == "record_success":
        return JSON.validate_python(record_success(input_value))
    if input_value["function"] == "request_input":
        return parsed(request_input(input_value["data"]))
    if input_value["function"] == "response_output":
        return parsed(response_text(input_value))
    return parsed(stream_output(relayed(input_value["chunks"]), input_value["data"]))
