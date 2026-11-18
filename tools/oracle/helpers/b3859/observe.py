import json
from collections.abc import Mapping
from typing import TypedDict

from opentelemetry.sdk.trace import ReadableSpan
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.util.types import AttributeValue
from pydantic import JsonValue, TypeAdapter, ValidationError

DICT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class SpanLine(TypedDict):
    name: str
    parent: str
    kind: str
    status: str
    start: str
    end: str
    attributes: dict[str, str]


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
