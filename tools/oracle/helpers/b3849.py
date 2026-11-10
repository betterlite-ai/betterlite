import sys
from collections.abc import Callable, Mapping, Sequence
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Fields:
    def __init__(self, **fields: object) -> None:
        for name, value in fields.items():
            setattr(self, name, value)


class Resource:
    def __init__(self, attributes: Mapping[str, str] | None = None) -> None:
        self.attributes: dict[str, str] = dict(attributes or {})

    def merge(self, other: "Resource") -> "Resource":
        merged = dict(self.attributes)
        merged.update(other.attributes)
        return Resource(merged)


class SpanProcessor(Fields):
    def on_start(self, _span: object, _parent_context: object = None) -> None:
        return None

    def on_end(self, _span: object) -> None:
        return None

    def shutdown(self) -> None:
        return None

    def force_flush(self, _timeout_millis: int = 30000) -> bool:
        return True


class FakeStatus:
    def __init__(self, status_code: str, description: str | None) -> None:
        self.status_code = status_code
        self.description = description


class FakeParent:
    def __init__(self, span_id: int) -> None:
        self.span_id = span_id


class FakeSpan:
    def __init__(self, **fields: object) -> None:
        for name, value in fields.items():
            setattr(self, name, value)


class StatusLike(Protocol):
    status_code: object
    description: str | None


class ParentLike(Protocol):
    span_id: int


class ResourceLike(Protocol):
    attributes: Mapping[str, str]


class EventLike(Protocol):
    name: str
    attributes: Mapping[str, object]


class SpanLike(Protocol):
    name: str
    attributes: Mapping[str, object]
    events: Sequence[object]
    status: StatusLike
    parent: ParentLike | None
    resource: ResourceLike


def _module(name: str, **members: object) -> None:
    module = ModuleType(name)
    module.__path__ = []
    for member, value in members.items():
        setattr(module, member, value)
    sys.modules[name] = module


# providers.py imports the OTel SDK, which the pinned proxy extra leaves out
_module("opentelemetry.sdk")
_module("opentelemetry.sdk._logs", LoggerProvider=Fields)
_module(
    "opentelemetry.sdk._logs.export",
    BatchLogRecordProcessor=Fields,
    ConsoleLogExporter=Fields,
    InMemoryLogExporter=Fields,
    LogExporter=Fields,
    SimpleLogRecordProcessor=Fields,
)
_module("opentelemetry.sdk.metrics", MeterProvider=Fields)
_module("opentelemetry.sdk.metrics.export", MetricReader=Fields)
_module("opentelemetry.sdk.resources", Resource=Resource)
_module(
    "opentelemetry.sdk.trace",
    Event=Fields,
    ReadableSpan=Fields,
    SpanProcessor=SpanProcessor,
    TracerProvider=Fields,
    Span=Fields,
)
_module(
    "opentelemetry.sdk.trace.export",
    BatchSpanProcessor=Fields,
    ConsoleSpanExporter=Fields,
    SimpleSpanProcessor=Fields,
    SpanExporter=Fields,
)
_module("opentelemetry.sdk.trace.export.in_memory_span_exporter", InMemorySpanExporter=Fields)

from litellm.integrations.otel.model.destination import OtelDestination  # noqa: E402
from litellm.integrations.otel.plumbing import providers  # noqa: E402

ProviderFunction: TypeAlias = Callable[..., object]


def provider(name: str) -> ProviderFunction:
    return cast(ProviderFunction, vars(providers)[name])


_destination_class = cast(Callable[..., object], OtelDestination)
_sdk_event = cast(Callable[..., object], vars(sys.modules["opentelemetry.sdk.trace"])["Event"])
_for_destination = provider("_for_destination")
_scoped = provider("_scoped")
_in_scope = provider("_in_scope")
_is_llm_call = provider("is_llm_call_span")
_is_excluded = provider("_is_excluded_database_span")
_sink_key = provider("_sink_key")
_credential_name = provider("_credential_name")


def otel_value(value: Json) -> object:
    if isinstance(value, (str, bool, int, float)):
        return value
    if isinstance(value, list):
        return tuple(otel_value(item) for item in value)
    raise ValueError(f"unsupported span attribute value {type(value).__name__}")


def attribute_value(value: object) -> Json:
    if isinstance(value, (str, bool, int, float)):
        return value
    if isinstance(value, (list, tuple)):
        return [attribute_value(item) for item in cast(Sequence[object], value)]
    raise ValueError(f"unsupported span attribute value {type(value).__name__}")


def attributes_json(attributes: Mapping[str, object]) -> dict[str, Json]:
    return {key: attribute_value(value) for key, value in attributes.items()}


def event_from(value: Json) -> object:
    fields = cast(dict[str, Json], value)
    attributes = cast(dict[str, Json], fields.get("attributes") or {})
    return _sdk_event(
        name=cast(str, fields.get("name", "exception")),
        attributes={key: otel_value(item) for key, item in attributes.items()},
        timestamp=None,
    )


def span_from(value: Json) -> FakeSpan:
    fields = cast(dict[str, Json], value)
    attributes = cast(dict[str, Json], fields.get("attributes") or {})
    events = cast(list[Json], fields.get("events") or [])
    status = cast(dict[str, Json], fields.get("status") or {})
    resource = cast(dict[str, Json], fields.get("resource") or {})
    parent = cast(str | None, fields.get("parent"))
    return FakeSpan(
        name=cast(str, fields.get("name", "call model")),
        attributes={key: otel_value(item) for key, item in attributes.items()},
        events=tuple(event_from(item) for item in events),
        status=FakeStatus(
            cast(str, status.get("code", "unset")),
            cast(str | None, status.get("description")),
        ),
        parent=None if parent is None else FakeParent(int(parent, 16)),
        resource=Resource({key: cast(str, item) for key, item in resource.items()}),
        context=None,
        links=(),
        kind=None,
        start_time=None,
        end_time=None,
        instrumentation_scope=None,
    )


def destination_from(value: Json) -> object:
    fields = cast(dict[str, Json], value)
    return _destination_class(
        endpoint=cast(str, fields.get("endpoint")),
        headers=cast(dict[str, str], fields.get("headers") or {}),
        resource_attributes=cast(dict[str, str], fields.get("resource_attributes") or {}),
        callback_name=cast(str | None, fields.get("callback_name")),
        protocol=cast(str | None, fields.get("protocol")),
        span_scope=cast(str, fields.get("span_scope") or "full"),
    )


def event_json(event: object) -> Json:
    like = cast(EventLike, event)
    return {"name": like.name, "attributes": cast(Json, attributes_json(like.attributes))}


def status_json(status: object) -> Json:
    like = cast(StatusLike, status)
    return {"code": str(like.status_code), "description": like.description or ""}


def parent_json(parent: object | None) -> Json:
    if parent is None:
        return None
    return f"{cast(ParentLike, parent).span_id:016x}"


def view_json(span: object) -> dict[str, Json]:
    like = cast(SpanLike, span)
    return {
        "name": like.name,
        "attributes": cast(Json, attributes_json(like.attributes)),
        "events": cast(Json, [event_json(event) for event in like.events]),
        "status": status_json(like.status),
        "parent": parent_json(like.parent),
    }


def run(input_value: dict[str, Json]) -> Json:
    return {"result": dispatch(input_value)}


def span_arg(input_value: dict[str, Json]) -> object:
    return span_from(input_value.get("span"))


def scope_arg(input_value: dict[str, Json]) -> str:
    return cast(str, input_value.get("span_scope") or "full")


def dispatch(input_value: dict[str, Json]) -> Json:  # noqa: PLR0911
    function = input_value.get("function")
    if function == "for_destination":
        destination = destination_from(input_value.get("destination"))
        view = _for_destination(span_arg(input_value), destination)
        return cast(Json, {**view_json(view), "resource": resource_json(view)})
    if function == "scoped":
        return cast(Json, view_json(_scoped(span_arg(input_value), scope_arg(input_value))))
    if function == "in_scope":
        return cast(Json, _in_scope(span_arg(input_value), scope_arg(input_value)))
    if function == "is_llm_call_span":
        return cast(Json, _is_llm_call(span_arg(input_value)))
    if function == "is_excluded_database_span":
        return excluded_json(input_value)
    if function == "sink_key":
        return sink_json(input_value)
    if function == "credential_name":
        return cast(Json, _credential_name(cast(str, input_value.get("header"))))
    raise ValueError(f"unknown b3849 function {function} ({litellm.__name__})")


def excluded_json(input_value: dict[str, Json]) -> Json:
    span = cast(SpanLike, span_arg(input_value))
    excluded = frozenset(cast(list[str], input_value.get("excluded") or []))
    return cast(Json, _is_excluded(span.attributes, excluded))


def sink_json(input_value: dict[str, Json]) -> Json:
    endpoint = cast(str | None, input_value.get("endpoint"))
    headers = cast(dict[str, str], input_value.get("headers") or {})
    sink = cast(tuple[str, tuple[tuple[str, str], ...]] | None, _sink_key(endpoint, headers))
    if sink is None:
        return None
    return [sink[0], [[name, value] for name, value in sink[1]]]


def resource_json(span: object) -> dict[str, Json]:
    like = cast(SpanLike, span)
    return {key: cast(Json, value) for key, value in like.resource.attributes.items()}
