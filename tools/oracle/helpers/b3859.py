import asyncio
import json
from collections.abc import Callable, Mapping
from datetime import UTC, datetime
from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.integrations.opentelemetry import OpenTelemetry, OpenTelemetryConfig
from litellm.integrations.opentelemetry_utils.gen_ai_semconv import OTELSemconvCategory
from litellm.types.utils import StandardCallbackDynamicParams
from opentelemetry.sdk.trace import ReadableSpan, TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import Span, Tracer, use_span
from opentelemetry.util.types import AttributeValue
from pydantic import JsonValue, TypeAdapter, ValidationError
from typing_extensions import override

PARENT_SPAN_KEY = "litellm_parent_otel_span"
DICT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

# the parent span travels as a live Span object, so one metadata level holds a non-JSON value
CarriedMetadata: TypeAlias = dict[str, JsonValue | Span]
CarriedParams: TypeAlias = dict[str, JsonValue | CarriedMetadata]
CarriedKwargs: TypeAlias = dict[str, JsonValue | CarriedMetadata | CarriedParams]


class Parent(TypedDict):
    name: str
    carrier: str
    start: NotRequired[float]
    end: NotRequired[float]


class Common(TypedDict):
    event: Literal["success", "failure"]
    kwargs: dict[str, JsonValue]
    start_time: float
    end_time: float
    callback_name: NotRequired[str | None]
    parent: NotRequired[Parent]
    dynamic_config: NotRequired[bool]
    dynamic_headers: NotRequired[dict[str, str]]
    config_headers: NotRequired[str]
    semconv: NotRequired[bool]
    repeat: NotRequired[bool]


class LogCall(Common):
    function: Literal["log"]


class TracerCall(Common):
    function: Literal["tracer"]


class GuardrailCall(Common):
    function: Literal["guardrails"]
    request_data: dict[str, JsonValue]


Scenario: TypeAlias = LogCall | TracerCall | GuardrailCall


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
        self,
        # ast-grep-ignore: no-long-names
        standard_callback_dynamic_params: StandardCallbackDynamicParams,
    ) -> dict[str, str] | None:
        return self.dynamic_headers

    @override
    def construct_dynamic_otel_config(
        self,
        # ast-grep-ignore: no-long-names
        standard_callback_dynamic_params: StandardCallbackDynamicParams,
    ) -> OpenTelemetryConfig | None:
        return self.dynamic_config


def probe_logger(scenario: Common, provider: TracerProvider) -> Probe:
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
    logger = Probe(
        config=config,
        callback_name=scenario.get("callback_name"),
        tracer_provider=provider,
    )
    logger.probe_calls = []
    logger.probe_tracer = provider.get_tracer("b3859")
    logger.dynamic_headers = scenario.get("dynamic_headers")
    dynamic_config = scenario.get("dynamic_config")
    logger.dynamic_config = OpenTelemetryConfig() if dynamic_config is True else None
    return logger


def parent_span(scenario: Common, provider: TracerProvider) -> Span | None:
    spec = scenario.get("parent")
    if spec is None:
        return None
    start = spec.get("start")
    start_ns = int(scenario["start_time"] * 1e9) if start is None else int(start * 1e9)
    return provider.get_tracer("b3859").start_span(spec["name"], start_time=start_ns)


def end_parent(scenario: Common, parent: Span | None) -> None:
    spec = scenario.get("parent")
    if parent is None or spec is None:
        return
    end = spec.get("end")
    end_ns = int(scenario["end_time"] * 1e9) if end is None else int(end * 1e9)
    parent.end(end_time=end_ns)


def with_carrier(kwargs: CarriedKwargs, parent: Span) -> CarriedKwargs:
    params = DICT.validate_python(kwargs["litellm_params"])
    metadata = DICT.validate_python(params["metadata"])
    carried: CarriedMetadata = {**metadata, PARENT_SPAN_KEY: parent}
    params_carried: CarriedParams = {**params, "metadata": carried}
    return {**kwargs, "litellm_params": params_carried}


def emit(logger: Probe, scenario: Common, kwargs: CarriedKwargs) -> None:
    start = datetime.fromtimestamp(scenario["start_time"], tz=UTC)
    end = datetime.fromtimestamp(scenario["end_time"], tz=UTC)
    if scenario["event"] == "success":
        logger.log_success_event(  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L756
            kwargs, None, start, end
        )
    else:
        logger.log_failure_event(  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L759
            kwargs, None, start, end
        )
    if scenario.get("repeat") is True:
        asyncio.run(async_emit(logger, scenario, kwargs, start, end))


async def async_emit(
    logger: Probe, scenario: Common, kwargs: CarriedKwargs, start: datetime, end: datetime
) -> None:
    if scenario["event"] == "success":
        await logger.async_log_success_event(kwargs, None, start, end)  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L762
    else:
        await logger.async_log_failure_event(kwargs, None, start, end)  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L765


def lifecycle_call(scenario: Common, logger: Probe, parent: Span | None) -> None:
    kwargs: CarriedKwargs = {**scenario["kwargs"]}
    spec = scenario.get("parent")
    carrier = "" if spec is None else spec["carrier"]
    if carrier == "metadata" and parent is not None:
        emit(logger, scenario, with_carrier(kwargs, parent))
        return
    if carrier == "ambient" and parent is not None:
        with use_span(parent, end_on_exit=False):
            emit(logger, scenario, kwargs)
        return
    emit(logger, scenario, kwargs)


def guardrails_call(scenario: GuardrailCall, logger: Probe) -> None:
    request_data = DICT.validate_python(scenario["request_data"])
    bucket = "litellm_metadata" if "litellm_metadata" in request_data else "metadata"
    metadata = DICT.validate_python(request_data[bucket])
    request_data[bucket] = metadata
    params = DICT.validate_python(scenario["kwargs"]["litellm_params"])
    params_carried: CarriedParams = {**params, "metadata": metadata}
    kwargs: CarriedKwargs = {**scenario["kwargs"], "litellm_params": params_carried}
    emit(logger, scenario, kwargs)
    logger._emit_guardrail_spans_from_request_data(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L991-L1029
        request_data=request_data,
        parent_span=None,
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
        case None | bool() as flag:
            return "true" if flag else "false"
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
    values = DICT.validate_python(attributes)
    return {key: value_text(value) for key, value in values.items()}


def line_of(span: ReadableSpan, names: Mapping[int, str]) -> SpanLine:
    parent_context = span.parent
    parent_id = 0 if parent_context is None else parent_context.span_id
    parent_name = ""
    if parent_id in names:
        parent_name = names[parent_id]
    return SpanLine(
        name=span.name,
        parent=parent_name,
        kind=span.kind.name,
        status=span.status.status_code.name,
        start=str(span.start_time),
        end=str(span.end_time),
        attributes=attribute_map(span.attributes),
    )


def read_spans(exporter: InMemorySpanExporter) -> list[SpanLine]:
    spans = exporter.get_finished_spans()
    names: dict[int, str] = {}
    for span in spans:
        context = span.get_span_context()
        if context is not None:
            names[context.span_id] = span.name
    lines = [line_of(span, names) for span in spans]
    rows: list[tuple[int, int, str, str, int]] = [
        (int(line["start"]), int(line["end"]), line["name"], line["parent"], index)
        for index, line in enumerate(lines)
    ]
    rows.sort()
    return [lines[row[4]] for row in rows]


def choice_of(scenario: Common, logger: Probe) -> str:
    if not logger.probe_calls:
        return "global"
    if logger.probe_calls[0].startswith("dynamic_config:"):
        return "dynamic_config"
    if "standard_callback_dynamic_params" not in scenario["kwargs"]:
        return "env_headers"
    return "dynamic_headers"


def run(input_value: Scenario) -> JsonValue:
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    logger = probe_logger(input_value, provider)
    parent = parent_span(input_value, provider)
    if input_value["function"] == "tracer":
        logger.get_tracer_to_use_for_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L1057
            input_value["kwargs"]
        )
    elif input_value["function"] == "guardrails":
        guardrails_call(input_value, logger)
    else:
        lifecycle_call(input_value, logger, parent)
    end_parent(input_value, parent)
    spans = read_spans(exporter)
    return JSON.validate_python(Observed(choice=choice_of(input_value, logger), spans=spans))
