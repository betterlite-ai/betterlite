import os
import sys
from collections import OrderedDict
from collections.abc import Callable, Mapping
from threading import Lock
from types import ModuleType
from typing import Protocol, TypeAlias, cast
from urllib import parse

import litellm
from litellm.integrations.opentelemetry import (
    _PROVIDER_SHUTDOWN_EXECUTOR,  # pyright: ignore[reportPrivateUsage]
    OpenTelemetry,
    OpenTelemetryConfig,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

SERVICE_NAME = "service.name"
RESOURCE_ATTRIBUTES_ENV = "OTEL_RESOURCE_ATTRIBUTES"
SERVICE_NAME_ENV = "OTEL_SERVICE_NAME"
DEFAULT_MAX_DYNAMIC_TRACER_PROVIDERS = 256

_RECORDED_SHUTDOWNS: list[int] = []


class ResourceLike(Protocol):
    @property
    def attributes(self) -> dict[str, str]: ...


class _Exporter:
    def __init__(
        self,
        kind: str,
        endpoint: str | None = None,
        headers: Mapping[str, str] | None = None,
    ) -> None:
        self.kind = kind
        self.endpoint = endpoint
        self.headers: dict[str, str] = dict(headers or {})


class InMemorySpanExporter(_Exporter):
    def __init__(self) -> None:
        super().__init__("shared")

    def export(self, *_batch: object) -> None: ...


class ConsoleSpanExporter(_Exporter):
    def __init__(self) -> None:
        super().__init__("console")


class _OtlpExporter(_Exporter):
    def __init__(
        self,
        *,
        endpoint: str | None = None,
        headers: Mapping[str, str] | None = None,
        **_: object,
    ) -> None:
        super().__init__("otlp_http", endpoint, headers)


class _OtlpGrpcExporter(_Exporter):
    def __init__(
        self,
        *,
        endpoint: str | None = None,
        headers: Mapping[str, str] | None = None,
        **_: object,
    ) -> None:
        super().__init__("otlp_grpc", endpoint, headers)


class ConsoleLogExporter(_Exporter):
    def __init__(self) -> None:
        super().__init__("console")


class ConsoleMetricExporter(_Exporter):
    def __init__(self) -> None:
        super().__init__("console")


class SimpleSpanProcessor:
    def __init__(self, exporter: _Exporter) -> None:
        self.exporter = exporter


class BatchSpanProcessor(SimpleSpanProcessor): ...


class SpanExporter: ...


class PeriodicExportingMetricReader:
    def __init__(self, exporter: _Exporter, export_interval_millis: int | None = None) -> None:
        self.exporter = exporter
        self.export_interval_millis = export_interval_millis


class Resource:
    def __init__(self, attributes: Mapping[str, str | None]) -> None:
        self._attributes = {
            key: value for key, value in attributes.items() if key and value is not None
        }

    @property
    def attributes(self) -> dict[str, str]:
        return dict(self._attributes)

    @staticmethod
    def create(attributes: Mapping[str, str | None] | None = None) -> "Resource":
        merged = OTELResourceDetector().detect().attributes
        merged.update(
            {key: value for key, value in (attributes or {}).items() if key and value is not None}
        )
        if not merged.get(SERVICE_NAME):
            merged[SERVICE_NAME] = "unknown_service"
        return Resource(merged)

    def merge(self, other: "Resource") -> "Resource":
        return Resource({**self.attributes, **other.attributes})


class OTELResourceDetector:
    def detect(self) -> "Resource":
        attributes: dict[str, str] = {}
        pairs = os.environ.get(RESOURCE_ATTRIBUTES_ENV, "")
        if pairs:
            for item in pairs.split(","):
                key, separator, value = item.partition("=")
                if not separator:
                    continue
                attributes[key.strip()] = parse.unquote(value.strip())
        service_name = os.environ.get(SERVICE_NAME_ENV, "")
        if service_name:
            attributes[SERVICE_NAME] = service_name
        return Resource(attributes)


def _install(name: str, **attributes: object) -> None:
    module = ModuleType(name)
    module.__path__ = []
    for attribute_name, value in attributes.items():
        setattr(module, attribute_name, value)
    sys.modules[name] = module


def _install_sdk_stubs() -> None:
    for name in (
        "opentelemetry.sdk",
        "opentelemetry.sdk.trace",
        "opentelemetry.sdk._logs",
        "opentelemetry.sdk.metrics",
        "opentelemetry.exporter",
        "opentelemetry.exporter.otlp",
        "opentelemetry.exporter.otlp.proto",
        "opentelemetry.exporter.otlp.proto.http",
        "opentelemetry.exporter.otlp.proto.grpc",
    ):
        _install(name)
    _install(
        "opentelemetry.sdk.trace.export",
        BatchSpanProcessor=BatchSpanProcessor,
        ConsoleSpanExporter=ConsoleSpanExporter,
        SimpleSpanProcessor=SimpleSpanProcessor,
        SpanExporter=SpanExporter,
    )
    _install(
        "opentelemetry.sdk.trace.export.in_memory_span_exporter",
        InMemorySpanExporter=InMemorySpanExporter,
    )
    _install("opentelemetry.sdk._logs.export", ConsoleLogExporter=ConsoleLogExporter)
    _install(
        "opentelemetry.sdk.metrics.export",
        ConsoleMetricExporter=ConsoleMetricExporter,
        PeriodicExportingMetricReader=PeriodicExportingMetricReader,
    )
    _install(
        "opentelemetry.sdk.resources",
        OTELResourceDetector=OTELResourceDetector,
        Resource=Resource,
    )
    _install(
        "opentelemetry.exporter.otlp.proto.http.trace_exporter", OTLPSpanExporter=_OtlpExporter
    )
    _install("opentelemetry.exporter.otlp.proto.http._log_exporter", OTLPLogExporter=_OtlpExporter)
    _install(
        "opentelemetry.exporter.otlp.proto.http.metric_exporter",
        OTLPMetricExporter=_OtlpExporter,
    )
    _install(
        "opentelemetry.exporter.otlp.proto.grpc.trace_exporter",
        OTLPSpanExporter=_OtlpGrpcExporter,
    )
    _install(
        "opentelemetry.exporter.otlp.proto.grpc._log_exporter",
        OTLPLogExporter=_OtlpGrpcExporter,
    )
    _install(
        "opentelemetry.exporter.otlp.proto.grpc.metric_exporter",
        OTLPMetricExporter=_OtlpGrpcExporter,
    )


_install_sdk_stubs()


class RecordingProvider:
    def __init__(self, index: int) -> None:
        self.index = index

    def get_tracer(self, name: str) -> str:
        return name

    def shutdown(self) -> None:
        _RECORDED_SHUTDOWNS.append(self.index)


class RecordingBuild:
    def __init__(self, index: int) -> None:
        self.index = index
        self.built = False

    def build(self) -> RecordingProvider:
        self.built = True
        return RecordingProvider(self.index)


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def bool_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def config_from(fields: dict[str, Json]) -> OpenTelemetryConfig:
    exporter = fields.get("exporter")
    exporter_value: object = "console" if exporter is None else exporter
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


def new_logger(config: OpenTelemetryConfig) -> OpenTelemetry:
    logger = OpenTelemetry.__new__(OpenTelemetry)
    logger.config = config
    logger.callback_name = None
    logger.OTEL_EXPORTER = cast("str", config.exporter)
    logger.OTEL_ENDPOINT = config.endpoint
    logger.OTEL_HEADERS = config.headers
    return logger


def configured_logger(input_value: dict[str, Json]) -> OpenTelemetry:
    logger = new_logger(config_from(object_value(input_value.get("config", {}), "config")))
    logger.callback_name = cast("str | None", input_value.get("callback_name"))
    return logger


def exporter_value(exporter: _Exporter) -> Json:
    return cast(
        Json,
        {"kind": exporter.kind, "endpoint": exporter.endpoint, "headers": exporter.headers},
    )


def endpoint_value(input_value: dict[str, Json]) -> Json:
    logger = new_logger(OpenTelemetryConfig())
    logger.callback_name = cast("str | None", input_value.get("callback_name"))
    return cast(
        Json,
        logger._normalize_otel_endpoint(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast("str | None", input_value.get("endpoint")),
            string_value(input_value.get("signal"), "signal"),
        ),
    )


def headers_value(input_value: dict[str, Json]) -> Json:
    value = input_value.get("headers")
    headers: object = None
    if isinstance(value, str):
        headers = value
    elif isinstance(value, dict):
        headers = {name: string_value(item, "header value") for name, item in value.items()}
    elif value is not None:
        raise ValueError("headers must be a string, an object or absent")
    return cast(
        Json,
        OpenTelemetry._get_headers_dictionary(headers),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )


def parsed_headers(value: str | Mapping[str, str] | None) -> dict[str, str]:
    # _get_span_processor keeps headers private for a console exporter, so parse them here.
    return OpenTelemetry._get_headers_dictionary(value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def span_plan_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    override = input_value.get("config_override")
    dynamic_headers = input_value.get("dynamic_headers")
    override_config = (
        None if override is None else config_from(object_value(override, "config_override"))
    )
    chosen_headers = logger.OTEL_HEADERS if override_config is None else override_config.headers
    dynamic_map = (
        cast("dict[str, str]", dynamic_headers) if isinstance(dynamic_headers, dict) else None
    )
    headers = parsed_headers(dynamic_map or chosen_headers)
    processor = cast(
        SimpleSpanProcessor,
        logger._get_span_processor(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            dynamic_headers=dynamic_map,
            config_override=override_config,
        ),
    )
    value = cast("dict[str, Json]", exporter_value(processor.exporter))
    value["headers"] = cast(Json, headers)
    value["processor"] = "batch" if isinstance(processor, BatchSpanProcessor) else "simple"
    return cast(Json, value)


def log_plan_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    headers = parsed_headers(logger.OTEL_HEADERS)
    exporter = cast(
        _Exporter,
        logger._get_log_exporter(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    value = cast("dict[str, Json]", exporter_value(exporter))
    value["headers"] = cast(Json, headers)
    return cast(Json, value)


def metric_plan_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    headers = parsed_headers(logger.OTEL_HEADERS)
    reader = cast(
        PeriodicExportingMetricReader,
        logger._get_metric_reader(),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    value = cast("dict[str, Json]", exporter_value(reader.exporter))
    value["headers"] = cast(Json, headers)
    return cast(Json, value)


def cache_key_text(key: dict[str, Json]) -> str:
    if "config" in key:
        config = object_value(key["config"], "key.config")
        return (
            f"dynamic_config:{config.get('exporter')}:"
            f"{config.get('endpoint')}:{config.get('headers')}"
        )
    headers = cast("dict[str, str]", object_value(key["headers"], "key.headers"))
    return str(sorted(headers.items()))


def key_value_of(requests: list[Json], index: int) -> Json:
    request = object_value(requests[index], "request")
    return request.get("key")


def tracer_cache_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    providers = cast(
        int, input_value.get("max_providers", DEFAULT_MAX_DYNAMIC_TRACER_PROVIDERS)
    )
    setattr(logger, "_max_dynamic_tracer_providers", max(1, providers))  # noqa: B010
    logger._tracer_provider_cache = OrderedDict()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    setattr(logger, "_tracer_provider_cache_lock", Lock())  # noqa: B010
    logger._litellm_resource_memo = None  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    built: list[Json] = []
    key_of: dict[str, Json] = {}
    requests = cast("list[Json]", input_value.get("requests", []))
    for index, request_value in enumerate(requests):
        request = object_value(request_value, "request")
        key_value = request.get("key")
        if key_value is None:
            raise ValueError("every request needs a key")
        key_text = cache_key_text(object_value(key_value, "request.key"))
        key_of[key_text] = key_value
        recorder = RecordingBuild(index)
        logger._cached_dynamic_tracer(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            key_text,
            recorder.build,
            bool_value(request.get("owns_exporter"), "owns_exporter"),
        )
        built.append(recorder.built)
    _PROVIDER_SHUTDOWN_EXECUTOR.shutdown(wait=True)
    cache = logger._tracer_provider_cache  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    order = [key_of[text] for text in cache]
    shut_down = [key_value_of(requests, index) for index in sorted(_RECORDED_SHUTDOWNS)]
    return cast(Json, {"built": built, "order": order, "shut_down": shut_down})


def resource_value(input_value: dict[str, Json]) -> Json:
    logger = configured_logger(input_value)
    resource = cast(
        ResourceLike,
        OpenTelemetry._get_litellm_resource(logger.config),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    )
    attributes = {
        name: value
        for name, value in resource.attributes.items()
        if not name.startswith("telemetry.sdk.")
    }
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
