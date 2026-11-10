import sys
from collections.abc import Callable, Mapping, Sequence
from types import ModuleType
from typing import TypeAlias, cast

import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Fields:
    def __init__(self, **fields: object) -> None:
        for name, value in fields.items():
            setattr(self, name, value)


class Resource:
    attributes: dict[str, str]

    def __init__(self, attributes: Mapping[str, str] | None = None) -> None:
        self.attributes = dict(attributes or {})

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


class FakeSpan(Fields):
    pass


class SpanRecord:
    name: str
    attributes: Mapping[str, object]


def _module(name: str, **members: object) -> None:
    module = ModuleType(name)
    module.__path__ = []
    for member, value in members.items():
        setattr(module, member, value)
    sys.modules[name] = module


# providers.py imports the OTel SDK, which the pinned proxy extra leaves out
_SDK: dict[str, dict[str, object]] = {
    "opentelemetry.sdk": {},
    "opentelemetry.sdk._logs": {"LoggerProvider": Fields},
    "opentelemetry.sdk._logs.export": {
        "BatchLogRecordProcessor": Fields,
        "ConsoleLogExporter": Fields,
        "InMemoryLogExporter": Fields,
        "LogExporter": Fields,
        "SimpleLogRecordProcessor": Fields,
    },
    "opentelemetry.sdk.metrics": {"MeterProvider": Fields},
    "opentelemetry.sdk.metrics.export": {"MetricReader": Fields},
    "opentelemetry.sdk.resources": {"Resource": Resource},
    "opentelemetry.sdk.trace": {
        "Event": Fields,
        "ReadableSpan": Fields,
        "SpanProcessor": SpanProcessor,
        "TracerProvider": Fields,
        "Span": Fields,
    },
    "opentelemetry.sdk.trace.export": {
        "BatchSpanProcessor": Fields,
        "ConsoleSpanExporter": Fields,
        "SimpleSpanProcessor": Fields,
        "SpanExporter": Fields,
    },
    "opentelemetry.sdk.trace.export.in_memory_span_exporter": {"InMemorySpanExporter": Fields},
}
for _sdk_module, _members in _SDK.items():
    _module(_sdk_module, **_members)

from litellm.integrations.otel.model.destination import OtelDestination  # noqa: E402
from litellm.integrations.otel.plumbing import providers  # noqa: E402

Provider: TypeAlias = Callable[..., object]
_destination = cast(Provider, OtelDestination)
_for_destination = cast(Provider, vars(providers)["_for_destination"])
_sink_key = cast(Provider, vars(providers)["_sink_key"])


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


def span_from(value: Json) -> FakeSpan:
    fields = cast(dict[str, Json], value)
    unknown = set(fields) - {"name", "attributes"}
    if unknown:
        raise ValueError(f"unknown span fields {sorted(unknown)}")
    attributes = cast(dict[str, Json], fields.get("attributes") or {})
    return FakeSpan(
        name=fields.get("name"),
        attributes={key: otel_value(item) for key, item in attributes.items()},
        events=(),
        status=Fields(status_code="unset", description=None),
        parent=None,
        resource=Resource(),
        context=None,
        links=(),
        kind=None,
        start_time=None,
        end_time=None,
        instrumentation_scope=None,
    )


def record_json(span: object) -> Json:
    like = cast(SpanRecord, span)
    return {"name": like.name, "attributes": cast(Json, attributes_json(like.attributes))}


def run(input_value: dict[str, Json]) -> Json:
    unknown = set(input_value) - {"function", "span", "endpoint", "headers"}
    if unknown:
        raise ValueError(f"unknown input fields {sorted(unknown)}")
    return {"result": dispatch(input_value)}


def dispatch(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    span = span_from(input_value.get("span") or {})
    if function == "for_destination":
        destination = _destination(
            endpoint=input_value.get("endpoint") or "",
            headers=input_value.get("headers") or {},
            resource_attributes={},
            callback_name=None,
            protocol=None,
            span_scope="full",
        )
        return record_json(_for_destination(span, destination))
    if function == "sink_key":
        return sink_json(input_value)
    raise ValueError(f"unknown b3849 function {function} ({litellm.__name__})")


def sink_json(input_value: dict[str, Json]) -> Json:
    endpoint = cast(str | None, input_value.get("endpoint"))
    headers = cast(dict[str, str], input_value.get("headers") or {})
    sink = cast(tuple[str, tuple[tuple[str, str], ...]] | None, _sink_key(endpoint, headers))
    if sink is None:
        return None
    return cast(Json, [sink[0], [[name, value] for name, value in sink[1]]])
