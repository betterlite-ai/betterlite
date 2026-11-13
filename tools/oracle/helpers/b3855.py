import os
import sys
from collections import OrderedDict
from collections.abc import Callable, Mapping
from functools import partial
from threading import Lock
from types import ModuleType
from typing import TypeAlias, cast
from urllib import parse

import litellm
from litellm.integrations.opentelemetry import (
    _PROVIDER_SHUTDOWN_EXECUTOR,  # pyright: ignore[reportPrivateUsage]
    OpenTelemetry,
    OpenTelemetryConfig,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
DEFAULT_MAX_DYNAMIC_TRACER_PROVIDERS = 256

_RECORDED_SHUTDOWNS: list[int] = []
_normalize_endpoint = OpenTelemetry._normalize_otel_endpoint  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_headers_dictionary = OpenTelemetry._get_headers_dictionary  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_span_processor = OpenTelemetry._get_span_processor  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_log_plan = OpenTelemetry._get_log_exporter  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_metric_plan = OpenTelemetry._get_metric_reader  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_litellm_resource = OpenTelemetry._get_litellm_resource  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
_cached_tracer = OpenTelemetry._cached_dynamic_tracer  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]


class _Exporter:
    kind = "console"

    def __init__(
        self,
        endpoint: str | None = None,
        headers: Mapping[str, str] | None = None,
        **_: object,
    ) -> None:
        self.endpoint = endpoint
        self.headers: dict[str, str] = dict(headers or {})


class InMemorySpanExporter(_Exporter):
    kind = "shared"

    def export(self, *_batch: object) -> None: ...


class _OtlpHttpExporter(_Exporter):
    kind = "otlp_http"


class _OtlpGrpcExporter(_Exporter):
    kind = "otlp_grpc"


class SimpleSpanProcessor:
    def __init__(self, exporter: _Exporter) -> None:
        self.exporter = exporter


class BatchSpanProcessor(SimpleSpanProcessor): ...


class PeriodicExportingMetricReader:
    def __init__(self, exporter: _Exporter, export_interval_millis: int | None = None) -> None:
        self.exporter = exporter
        self.export_interval_millis = export_interval_millis


class Resource:
    def __init__(self, attributes: Mapping[str, str | None]) -> None:
        self.attributes = {key: value for key, value in attributes.items() if value}

    @staticmethod
    def create(attributes: Mapping[str, str | None] | None = None) -> "Resource":
        merged = OTELResourceDetector().detect().attributes
        merged.update({key: value for key, value in (attributes or {}).items() if value})
        merged.setdefault("service.name", "unknown_service")
        return Resource(merged)

    def merge(self, other: "Resource") -> "Resource":
        return Resource(self.attributes | other.attributes)


class OTELResourceDetector:
    def detect(self) -> "Resource":
        attributes: dict[str, str] = {}
        for item in os.environ.get("OTEL_RESOURCE_ATTRIBUTES", "").split(","):
            key, separator, value = item.partition("=")
            if separator:
                attributes[key.strip()] = parse.unquote(value.strip())
        if service_name := os.environ.get("OTEL_SERVICE_NAME"):
            attributes["service.name"] = service_name
        return Resource(attributes)


def _install(name: str, attributes: Mapping[str, object]) -> None:
    module = sys.modules.get(name) or ModuleType(name)
    module.__path__ = []
    vars(module).update(attributes)
    sys.modules[name] = module


SDK = "opentelemetry.sdk"
OTLP = "opentelemetry.exporter.otlp.proto"
SDK_PACKAGES = (
    SDK, f"{SDK}.trace", f"{SDK}._logs", f"{SDK}.metrics",
    "opentelemetry.exporter", OTLP, f"{OTLP}.http", f"{OTLP}.grpc",
)
SDK_MODULES = (
    (f"{SDK}.trace.export", "BatchSpanProcessor", BatchSpanProcessor),
    (f"{SDK}.trace.export", "ConsoleSpanExporter", _Exporter),
    (f"{SDK}.trace.export", "SimpleSpanProcessor", SimpleSpanProcessor),
    (f"{SDK}.trace.export", "SpanExporter", _Exporter),
    (f"{SDK}.trace.export.in_memory_span_exporter", "InMemorySpanExporter", InMemorySpanExporter),
    (f"{SDK}._logs.export", "ConsoleLogExporter", _Exporter),
    (f"{SDK}.metrics.export", "ConsoleMetricExporter", _Exporter),
    (f"{SDK}.metrics.export", "PeriodicExportingMetricReader", PeriodicExportingMetricReader),
    (f"{SDK}.resources", "OTELResourceDetector", OTELResourceDetector),
    (f"{SDK}.resources", "Resource", Resource),
)


def _install_sdk_stubs() -> None:
    for name in SDK_PACKAGES:
        _install(name, {})
    for name, attribute, exporter in SDK_MODULES:
        _install(name, {attribute: exporter})
    for transport, exporter in (("http", _OtlpHttpExporter), ("grpc", _OtlpGrpcExporter)):
        names = (("trace", "OTLPSpanExporter"), ("_log", "OTLPLogExporter"))
        for kind, attribute in (*names, ("metric", "OTLPMetricExporter")):
            _install(f"{OTLP}.{transport}.{kind}_exporter", {attribute: exporter})


_install_sdk_stubs()


class RecordingProvider:
    def __init__(self, index: int, built: list[int]) -> None:
        self.index = index
        built.append(index)

    def get_tracer(self, name: str) -> str:
        return name

    def shutdown(self) -> None:
        _RECORDED_SHUTDOWNS.append(self.index)


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def config_from(fields: dict[str, Json]) -> OpenTelemetryConfig:
    exporter_value: object = fields.get("exporter") or "console"
    if exporter_value == "in_memory":
        exporter_value = InMemorySpanExporter()
    return OpenTelemetryConfig(
        exporter=cast("str", exporter_value),
        endpoint=cast("str | None", fields.get("endpoint")),
        headers=cast("str | None", fields.get("headers")),
        service_name=cast("str | None", fields.get("service_name")),
        deployment_environment=cast("str | None", fields.get("deployment_environment")),
        model_id=cast("str | None", fields.get("model_id")),
    )


def configured_logger(input_value: dict[str, Json]) -> OpenTelemetry:
    logger = OpenTelemetry.__new__(OpenTelemetry)
    config = config_from(object_value(input_value.get("config", {}), "config"))
    logger.config = config
    logger.OTEL_EXPORTER = cast("str", config.exporter)
    logger.OTEL_ENDPOINT = config.endpoint
    logger.OTEL_HEADERS = config.headers
    logger.callback_name = cast("str | None", input_value.get("callback_name"))
    return logger


def plan_value(exporter: _Exporter, headers: Mapping[str, str] | None) -> Json:
    fields: dict[str, Json] = {"kind": exporter.kind, "endpoint": exporter.endpoint}
    fields["headers"] = dict(headers or {})
    return cast(Json, fields)


def endpoint_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    endpoint = cast("str | None", input_value.get("endpoint"))
    signal = string_value(input_value.get("signal"), "signal")
    return cast(Json, _normalize_endpoint(logger, endpoint, signal))


def headers_value(input_value: dict[str, Json]) -> Json:
    value = input_value.get("headers")
    headers: object = None
    if isinstance(value, str):
        headers = value
    elif isinstance(value, dict):
        headers = {name: string_value(item, "header value") for name, item in value.items()}
    elif value is not None:
        raise ValueError("headers must be a string, an object or absent")
    return cast(Json, _headers_dictionary(headers))


def span_plan_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    override = input_value.get("config_override")
    dynamic_headers = input_value.get("dynamic_headers")
    override_config = None if override is None else config_from(object_value(override, "override"))
    chosen = logger.OTEL_HEADERS if override_config is None else override_config.headers
    dynamic_map = (
        cast("dict[str, str]", dynamic_headers) if isinstance(dynamic_headers, dict) else None
    )
    processor = cast(
        SimpleSpanProcessor,
        _span_processor(logger, dynamic_headers=dynamic_map, config_override=override_config),
    )
    value = cast(
        "dict[str, Json]",
        plan_value(processor.exporter, _headers_dictionary(dynamic_map or chosen)),
    )
    value["processor"] = "batch" if isinstance(processor, BatchSpanProcessor) else "simple"
    return cast(Json, value)


def log_plan_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    exporter = cast(_Exporter, _log_plan(logger))
    return plan_value(exporter, _headers_dictionary(logger.OTEL_HEADERS))


def metric_plan_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    reader = cast(PeriodicExportingMetricReader, _metric_plan(logger))
    return plan_value(reader.exporter, _headers_dictionary(logger.OTEL_HEADERS))


def cache_key_text(key: dict[str, Json]) -> str:
    if "config" in key:
        config = object_value(key["config"], "key.config")
        exporter, endpoint, headers = map(config.get, ("exporter", "endpoint", "headers"))
        return f"dynamic_config:{exporter}:{endpoint}:{headers}"
    headers = cast("dict[str, str]", object_value(key["headers"], "key.headers"))
    return str(sorted(headers.items()))


def tracer_cache_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    providers = cast(int, input_value.get("max_providers", DEFAULT_MAX_DYNAMIC_TRACER_PROVIDERS))
    setattr(logger, "_max_dynamic_tracer_providers", max(1, providers))  # noqa: B010
    logger._tracer_provider_cache = OrderedDict()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    setattr(logger, "_tracer_provider_cache_lock", Lock())  # noqa: B010
    built: list[int] = []
    key_of: dict[str, Json] = {}
    requests = cast("list[Json]", input_value.get("requests", []))
    for index, request_value in enumerate(requests):
        request = object_value(request_value, "request")
        key_value = object_value(request.get("key"), "request.key")
        key_text = cache_key_text(key_value)
        key_of[key_text] = key_value
        recorder = partial(RecordingProvider, index, built)
        _cached_tracer(logger, key_text, recorder, cast(bool, request.get("owns_exporter")))  # pyright: ignore[reportArgumentType]
    _PROVIDER_SHUTDOWN_EXECUTOR.shutdown(wait=True)
    cache = logger._tracer_provider_cache  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    order = [key_of[text] for text in cache]
    shut_down = [
        object_value(requests[i], "request").get("key") for i in sorted(_RECORDED_SHUTDOWNS)
    ]
    built_flags = [index in built for index in range(len(requests))]
    return cast(Json, {"built": built_flags, "order": order, "shut_down": shut_down})


def resource_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    resource = cast(Resource, _litellm_resource(logger.config))
    attributes = {k: v for k, v in resource.attributes.items() if "telemetry.sdk." not in k}
    return cast(Json, {"attributes": attributes})


_FUNCTIONS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "normalize_otel_endpoint": endpoint_value,
    "headers_dictionary": headers_value,
    "span_processor_plan": span_plan_value,
    "log_exporter_plan": log_plan_value,
    "metric_reader_plan": metric_plan_value,
    "tracer_cache_sequence": tracer_cache_value,
    "litellm_resource": resource_value,
}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    handler = _FUNCTIONS.get(function)
    if handler is None:
        raise ValueError(f"unknown {litellm.__name__} helper function {function}")
    return handler(input_value)
