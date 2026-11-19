import asyncio
from typing import TypedDict

from litellm.integrations.opentelemetry import OpenTelemetry, OpenTelemetryConfig
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SimpleSpanProcessor
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import Span
from pydantic import JsonValue

from b3859.observe import attribute_map

from . import hooks


class EventLine(TypedDict):
    name: str
    attributes: dict[str, str]


class SpanLine(TypedDict):
    name: str
    parent: str
    kind: str
    status: str
    start: str
    end: str
    attributes: dict[str, str]
    events: list[EventLine]


def start_parent(
    provider: TracerProvider, spec: hooks.Parent | None, default_start: float
) -> Span | None:
    if spec is None:
        return None
    start = spec.get("start")
    return provider.get_tracer("b3858").start_span(
        spec["name"], start_time=int((default_start if start is None else start) * 1e9)
    )


def end_parent(span: Span | None, spec: hooks.Parent | None, default_end: float) -> None:
    if span is None or spec is None:
        return
    end = spec.get("end")
    span.end(end_time=int((default_end if end is None else end) * 1e9))


def read_spans(exporter: InMemorySpanExporter) -> list[SpanLine]:
    spans = exporter.get_finished_spans()
    names: dict[int, str] = {0: ""}
    for span in spans:
        context = span.get_span_context()
        if context is not None:
            names[context.span_id] = span.name
    lines: list[SpanLine] = []
    for span in spans:
        parent_id = 0 if span.parent is None else span.parent.span_id
        lines.append(
            SpanLine(
                name=span.name,
                parent=names[parent_id],
                kind=span.kind.name,
                status=span.status.status_code.name,
                start=str(span.start_time),
                end=str(span.end_time),
                attributes=attribute_map(span.attributes),
                events=[
                    EventLine(name=event.name, attributes=attribute_map(event.attributes))
                    for event in span.events
                ],
            )
        )
    rows: list[tuple[int, int, str, str, int]] = [
        (int(line["start"]), int(line["end"]), line["name"], line["parent"], index)
        for index, line in enumerate(lines)
    ]
    rows.sort()
    return [lines[row[4]] for row in rows]


def run(input_value: hooks.Scenario) -> JsonValue:
    exporter = InMemorySpanExporter()
    provider = TracerProvider()
    provider.add_span_processor(SimpleSpanProcessor(exporter))
    logger = OpenTelemetry(
        config=OpenTelemetryConfig(exporter="in_memory", skip_set_global=True),
        tracer_provider=provider,
    )
    spec = input_value.get("parent")
    parent = start_parent(provider, spec, input_value["start_time"])
    match input_value["function"]:
        case "service_success":
            asyncio.run(hooks.service_call(logger, input_value, parent, mode="success"))
        case "service_failure":
            asyncio.run(hooks.service_call(logger, input_value, parent, mode="failure"))
        case "post_call_failure":
            asyncio.run(hooks.failure_call(logger, input_value, parent))
        case "post_call_success":
            key_spec = input_value.get("key_parent")
            key_parent = start_parent(provider, key_spec, input_value["start_time"])
            asyncio.run(hooks.success_call(logger, input_value, parent, key_parent))
            end_parent(key_parent, key_spec, input_value["end_time"])
        case "management_success":
            asyncio.run(hooks.management_call(logger, input_value, parent, mode="success"))
        case "management_failure":
            asyncio.run(hooks.management_call(logger, input_value, parent, mode="failure"))
        case "error_attributes":
            spec_exception = input_value.get("exception")
            exception = None if spec_exception is None else hooks.exception_of(spec_exception)
            logger.record_error_attributes_on_span(parent, exception, input_value["status_code"])
    end_parent(parent, spec, input_value["end_time"])
    return hooks.DICT.validate_python({"spans": read_spans(exporter)})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/opentelemetry.py#L1052
