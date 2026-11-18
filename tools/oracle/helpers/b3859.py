import asyncio
import json
from collections.abc import Callable, Mapping
from datetime import UTC, datetime
from typing import Literal, NotRequired, Required, TypedDict, cast, override

import litellm
from litellm.integrations.opentelemetry import OpenTelemetry, OpenTelemetryConfig
from litellm.integrations.opentelemetry_utils.gen_ai_semconv import OTELSemconvCategory
from litellm.types.utils import StandardCallbackDynamicParams
from opentelemetry.sdk.trace import ReadableSpan, TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import Span, Tracer, use_span
from opentelemetry.util.types import AttributeValue
from pydantic import JsonValue, TypeAdapter, ValidationError

PARENT_SPAN_KEY = "litellm_parent_otel_span"
DICT = TypeAdapter(dict[str, JsonValue])
ATTRS = TypeAdapter(dict[str, JsonValue])
JSON = TypeAdapter(JsonValue)


class Parent(TypedDict):
    name: str
    carrier: str
    start: NotRequired[float]
    end: NotRequired[float]


class Scenario(TypedDict, total=False):
    event: Required[Literal["success", "failure"]]
    function: Required[Literal["log", "tracer", "guardrails"]]
    kwargs: Required[dict[str, JsonValue]]
    request_data: dict[str, JsonValue]
    start_time: Required[float]
    end_time: Required[float]
    callback_name: str | None
    parent: Parent
    dynamic_config: bool
    dynamic_headers: dict[str, str]
    config_headers: str
    semconv: bool
    repeat: bool


class SpanLine(TypedDict):
    name: str
    parent: str
    kind: str
    status: str
    start: str
    end: str
    attributes: dict[str, str]


class Observed(TypedDict):
    choice: str
    spans: list[SpanLine]


class Probe(OpenTelemetry):
    probe_calls: list[str]
    probe_tracer: Tracer
    dynamic_headers: dict[str, str] | None
    dynamic_config: OpenTelemetryConfig | None

    @override
    def _cached_dynamic_tracer(
        self, cache_key: str, build: Callable[[], TracerProvider], owns_exporter: bool
    ) -> Tracer:
        self.probe_calls.append(cache_key)
        return self.probe_tracer

    @override
    def construct_dynamic_otel_headers(
        self, standard_callback_dynamic_params: StandardCallbackDynamicParams
    ) -> dict[str, str] | None:
        return self.dynamic_headers

    @override
    def construct_dynamic_otel_config(
        self, standard_callback_dynamic_params: StandardCallbackDynamicParams
    ) -> OpenTelemetryConfig | None:
        return self.dynamic_config


def probe_logger(scenario: Scenario, provider: TracerProvider) -> Probe:
    categories: set[OTELSemconvCategory] = set()
    if scenario.get("semconv") is True:
        categories.add(OTELSemconvCategory.GEN_AI_LATEST_EXPERIMENTAL)
    config = OpenTelemetryConfig(
        exporter="in_memory",
        headers=scenario.get("config_headers"),
        enable_metrics=False,
        enable_events=False,
        skip_set_global=True,
        semconv_stability_opt_in=categories,
    )
    logger = Probe(config=config, callback_name=scenario.get("callback_name"), tracer_provider=provider)
    logger.probe_calls = []
    logger.probe_tracer = provider.get_tracer("b3859")
    logger.dynamic_headers = scenario.get("dynamic_headers")
    logger.dynamic_config = OpenTelemetryConfig() if scenario.get("dynamic_config") is True else None
    return logger


def parent_span(scenario: Scenario, provider: TracerProvider) -> Span | None:
    spec = scenario.get("parent")
    if spec is None:
        return None
    start = spec.get("start")
    start_ns = int(scenario["start_time"] * 1e9) if start is None else int(start * 1e9)
    return provider.get_tracer("b3859").start_span(spec["name"], start_time=start_ns)


def end_parent(scenario: Scenario, parent: Span | None) -> None:
    spec = scenario.get("parent")
    if parent is None or spec is None:
        return
    end = spec.get("end")
    end_ns = int(scenario["end_time"] * 1e9) if end is None else int(end * 1e9)
    parent.end(end_time=end_ns)


def with_carrier(kwargs: dict[str, JsonValue], parent: Span) -> None:
    params = DICT.validate_python(kwargs["litellm_params"])
    metadata = DICT.validate_python(params["metadata"])
    metadata[PARENT_SPAN_KEY] = cast(JsonValue, parent)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L2992
    params["metadata"] = metadata
    kwargs["litellm_params"] = params


def emit(logger: Probe, scenario: Scenario, kwargs: dict[str, JsonValue]) -> None:
    start = datetime.fromtimestamp(scenario["start_time"], tz=UTC)
    end = datetime.fromtimestamp(scenario["end_time"], tz=UTC)
    repeat = scenario.get("repeat") is True
    if scenario["event"] == "success":
        logger.log_success_event(kwargs, None, start, end)  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L756
        if repeat:
            asyncio.run(logger.async_log_success_event(kwargs, None, start, end))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L762
        return
    logger.log_failure_event(kwargs, None, start, end)  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L759
    if repeat:
        asyncio.run(logger.async_log_failure_event(kwargs, None, start, end))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L765


def lifecycle_call(scenario: Scenario, logger: Probe, parent: Span | None) -> None:
    kwargs = scenario["kwargs"]
    spec = scenario.get("parent")
    carrier = "" if spec is None else spec["carrier"]
    if carrier == "metadata" and parent is not None:
        with_carrier(kwargs, parent)
    if carrier == "ambient" and parent is not None:
        with use_span(parent, end_on_exit=False):
            emit(logger, scenario, kwargs)
        return
    emit(logger, scenario, kwargs)


def guardrails_call(scenario: Scenario, logger: Probe) -> None:
    request_data = DICT.validate_python(scenario["request_data"])
    bucket = "litellm_metadata" if "litellm_metadata" in request_data else "metadata"
    metadata = DICT.validate_python(request_data[bucket])
    request_data[bucket] = metadata
    params = DICT.validate_python(scenario["kwargs"]["litellm_params"])
    params["metadata"] = metadata
    scenario["kwargs"]["litellm_params"] = params
    emit(logger, scenario, scenario["kwargs"])
    logger._emit_guardrail_spans_from_request_data(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L991
        request_data=request_data, parent_span=None
    )


def json_text(text: str) -> str:
    if not text.startswith(("{", "[")):
        return text
    try:
        parsed = JSON.validate_json(text)
    except ValidationError:
        return text
    return json.dumps(parsed, sort_keys=True, separators=(",", ":"))


def value_text(value: JsonValue) -> str:
    match value:
        case None:
            return ""
        case bool():
            return "true" if value else "false"
        case str():
            return json_text(value)
        case int() | float():
            return str(value)
        case list():
            return ",".join(value_text(item) for item in value)
        case dict():
            return json.dumps(value, sort_keys=True, separators=(",", ":"))


def attribute_map(attributes: Mapping[str, AttributeValue] | None) -> dict[str, str]:
    if attributes is None:
        return {}
    values = ATTRS.validate_python(attributes)
    return {key: value_text(value) for key, value in values.items()}


def line_of(span: ReadableSpan, names: dict[int, str]) -> SpanLine:
    parent_id = 0 if span.parent is None else span.parent.span_id
    return SpanLine(
        name=span.name,
        parent=names[parent_id] if parent_id in names else "",
        kind=span.kind.name,
        status=span.status.status_code.name,
        start=str(span.start_time),
        end=str(span.end_time),
        attributes=attribute_map(span.attributes),
    )


def sort_key(line: SpanLine) -> tuple[int, int, str, str]:
    return (int(line["start"]), int(line["end"]), line["name"], line["parent"])


def read_spans(exporter: InMemorySpanExporter) -> list[SpanLine]:
    spans = exporter.get_finished_spans()
    names = {span.context.span_id: span.name for span in spans}
    lines = [line_of(span, names) for span in spans]
    return sorted(lines, key=sort_key)


def choice_of(scenario: Scenario, logger: Probe) -> str:
    if not logger.probe_calls:
        return "global"
    if logger.probe_calls[0].startswith("dynamic_config:"):
        return "dynamic_config"
    if scenario.get("dynamic_headers") is None:
        return "env_headers"
    return "dynamic_headers"


def run(scenario: Scenario) -> JsonValue:
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    logger = probe_logger(scenario, provider)
    parent = parent_span(scenario, provider)
    if scenario["function"] == "tracer":
        logger.get_tracer_to_use_for_request(scenario["kwargs"])
    elif scenario["function"] == "guardrails":
        guardrails_call(scenario, logger)
    else:
        lifecycle_call(scenario, logger, parent)
    end_parent(scenario, parent)
    spans = read_spans(exporter)
    return JSON.validate_python(Observed(choice=choice_of(scenario, logger), spans=spans))
