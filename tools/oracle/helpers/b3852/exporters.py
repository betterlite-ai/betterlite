from typing import TypedDict

from litellm.integrations.otel.logger import OpenTelemetryV2
from litellm.integrations.otel.model.config import ExporterSpec, OpenTelemetryV2Config
from litellm.integrations.otel.plumbing.context import set_request_root_span
from litellm.integrations.otel.plumbing.providers import (
    build_tracer_provider,
    register_exporter_factory,
)
from opentelemetry.metrics import MeterProvider
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import SpanExporter
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import Tracer
from typing_extensions import override

from .wire import KIND, ExporterInput


class MemoryExporter(InMemorySpanExporter):
    def __init__(self) -> None:
        super().__init__()
        self.shutdowns = 0

    @override
    def shutdown(self) -> None:
        self.shutdowns += 1
        super().shutdown()


class MemoryFactory:
    def __init__(self) -> None:
        self.specs: list[ExporterSpec] = []
        self.exporters: list[MemoryExporter] = []

    def __call__(self, spec: ExporterSpec) -> SpanExporter:
        exporter = MemoryExporter()
        self.specs.append(spec)
        self.exporters.append(exporter)
        return exporter


def config_from(exporters: list[ExporterInput]) -> OpenTelemetryV2Config:
    config = OpenTelemetryV2Config()
    config.exporters = [ExporterSpec.model_validate(exporter) for exporter in exporters]
    return config


def factory_for() -> MemoryFactory:
    factory = MemoryFactory()
    register_exporter_factory(KIND, factory)
    return factory


def default_provider(config: OpenTelemetryV2Config) -> tuple[TracerProvider, MemoryExporter]:
    exporter = MemoryExporter()
    provider = build_tracer_provider(config, exporter=exporter, use_simple_processor=True)
    return provider, exporter


def build_logger(
    config: OpenTelemetryV2Config,
    callback_name: str | None,
    meter_provider: MeterProvider | None = None,
) -> tuple[OpenTelemetryV2, MemoryExporter]:
    provider, exporter = default_provider(config)
    logger = OpenTelemetryV2(
        config=config,
        callback_name=callback_name,
        tracer_provider=provider,
        meter_provider=meter_provider,
    )
    return logger, exporter


def anchor(provider: TracerProvider) -> Tracer:
    tracer = provider.get_tracer("litellm")
    set_request_root_span(tracer.start_span("root"))
    return tracer


class SpanRecord(TypedDict):
    name: str
    rooted: bool
    links: int


def span_records(exporters: list[MemoryExporter]) -> list[SpanRecord]:
    return [
        SpanRecord(name=span.name, rooted=span.parent is None, links=len(span.links))
        for exporter in exporters
        for span in exporter.get_finished_spans()
    ]
