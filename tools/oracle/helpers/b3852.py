"""The B3852 inference lifecycle and tenant tracer cache, driven through LiteLLM."""

import asyncio
import logging
from datetime import datetime, timezone
from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.integrations.otel.logger import OpenTelemetryV2
from litellm.integrations.otel.model.config import ExporterSpec, OpenTelemetryV2Config
from litellm.integrations.otel.plumbing.context import set_request_root_span
from litellm.integrations.otel.plumbing.providers import (
    build_tracer_provider,
    register_exporter_factory,
)
from litellm.integrations.otel.plumbing.routing import TenantTracerCache
from opentelemetry.metrics import MeterProvider
from opentelemetry.sdk.metrics import MeterProvider as SdkMeterProvider
from opentelemetry.sdk.trace import ReadableSpan, TracerProvider
from opentelemetry.sdk.trace.export import SpanExporter
from opentelemetry.sdk.trace.export.in_memory_span_exporter import InMemorySpanExporter
from opentelemetry.trace import Tracer
from pydantic import JsonValue as Json

Fields: TypeAlias = dict[str, Json]
Params: TypeAlias = dict[str, str]

KIND = "b3852_memory"
CALL_ID = "call-1"
MODEL = "gpt-4o"
START_SECONDS = 1_704_067_200.0
END_SECONDS = START_SECONDS + 1.5


class ExporterInput(TypedDict):
    kind: str
    endpoint: str | None
    headers: str | None
    owner: str | None
    use_simple_processor: bool


class Input(TypedDict):
    function: Literal[
        "pre_call",
        "mcp_tool_call",
        "metric_error",
        "close_dedup",
        "cache_eviction",
        "project_headers",
    ]
    callback_name: str | None
    anchored: bool
    exporters: list[ExporterInput]
    credentials: Params | None
    auth_metadata: Params | None
    spaces: list[str]
    project_name: str | None
    filter: Json


class MemoryExporter(InMemorySpanExporter):
    def __init__(self) -> None:
        super().__init__()
        self.shutdowns = 0

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


class ErrorCounter(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.ERROR)
        self.count = 0

    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.ERROR:
            self.count += 1


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


def text_object(params: Params) -> Fields:
    return {key: value for key, value in params.items()}


def payload_metadata(auth_metadata: Params | None, tool_name: str | None) -> Fields:
    metadata: Fields = {}
    if tool_name is not None:
        metadata["mcp_tool_call_metadata"] = {"name": tool_name}
    if auth_metadata is not None:
        metadata["user_api_key_auth_metadata"] = text_object(auth_metadata)
    return metadata


def payload_document(call_type: str, status: str, metadata: Fields) -> Fields:
    return {
        "id": CALL_ID,
        "litellm_call_id": CALL_ID,
        "trace_id": "trace-1",
        "session_id": "session-1",
        "call_type": call_type,
        "response_cost": 0.5,
        "status": status,
        "status_fields": {"llm_api_status": status, "guardrail_status": "not_run"},
        "total_tokens": 5,
        "prompt_tokens": 2,
        "completion_tokens": 3,
        "startTime": START_SECONDS,
        "endTime": END_SECONDS,
        "completionStartTime": START_SECONDS + 0.5,
        "response_time": 1.0,
        "model_map_information": {"model_map_key": "", "model_map_value": None},
        "model": MODEL,
        "api_base": "https://api.openai.com",
        "metadata": metadata,
        "saved_cache_cost": 0.0,
        "request_tags": [],
        "model_parameters": {},
        "hidden_params": {},
    }


def pre_call_details(credentials: Params | None, auth_metadata: Params | None) -> Fields:
    details: Fields = {
        "model": MODEL,
        "call_type": "completion",
        "litellm_call_id": CALL_ID,
    }
    if credentials is not None:
        details["standard_callback_dynamic_params"] = text_object(credentials)
    if auth_metadata is not None:
        details["litellm_params"] = {
            "metadata": {"user_api_key_auth_metadata": text_object(auth_metadata)}
        }
    return details


def close_kwargs(
    payload: Fields, metadata: Fields, credentials: Params | None
) -> Fields:
    kwargs: Fields = {
        "standard_logging_object": payload,
        "litellm_call_id": CALL_ID,
        "model": MODEL,
        "call_type": payload["call_type"],
        "optional_params": {"stream": False},
        "litellm_params": {"metadata": metadata},
        "api_call_start_time": START_SECONDS,
    }
    if credentials is not None:
        kwargs["standard_callback_dynamic_params"] = text_object(credentials)
    return kwargs


async def log_event(logger: OpenTelemetryV2, kwargs: Fields, failed: bool) -> None:
    started = datetime.fromtimestamp(START_SECONDS, tz=timezone.utc)
    ended = datetime.fromtimestamp(END_SECONDS, tz=timezone.utc)
    if failed:
        await logger.async_log_failure_event(kwargs, None, started, ended)
    else:
        await logger.async_log_success_event(kwargs, None, started, ended)


def span_records(exporters: list[MemoryExporter]) -> list[Json]:
    records: list[Json] = []
    for exporter in exporters:
        for span in exporter.get_finished_spans():
            records.append(span_record(span))
    return records


def span_record(span: ReadableSpan) -> Fields:
    return {
        "name": span.name,
        "rooted": span.parent is None,
        "links": len(span.links),
    }


def run_pre_call(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory = factory_for()
    logger, default_exporter = build_logger(config, input_value["callback_name"])
    if input_value["anchored"]:
        anchor(logger.tracer_provider)
    logger.log_pre_api_call(
        MODEL,
        [],
        pre_call_details(input_value["credentials"], input_value["auth_metadata"]),
    )
    builds_after_pre_call = len(factory.specs)
    metadata = payload_metadata(input_value["auth_metadata"], None)
    payload = payload_document("completion", "success", metadata)
    asyncio.run(
        log_event(logger, close_kwargs(payload, metadata, input_value["credentials"]), False)
    )
    return {
        "builds_after_pre_call": builds_after_pre_call,
        "builds_after_close": len(factory.specs),
        "routed_spans": span_records(factory.exporters),
        "default_spans": span_records([default_exporter]),
    }


def run_mcp_tool_call(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory = factory_for()
    logger, default_exporter = build_logger(config, input_value["callback_name"])
    metadata = payload_metadata(input_value["auth_metadata"], "search")
    payload = payload_document("call_mcp_tool", "success", metadata)
    asyncio.run(
        log_event(logger, close_kwargs(payload, metadata, input_value["credentials"]), False)
    )
    names = sorted(
        record_name(record) for record in span_records([default_exporter])
    )
    return {"names": names, "count": len(names), "builds": len(factory.specs)}


def record_name(record: Json) -> str:
    if not isinstance(record, dict):
        raise ValueError("a span record is an object")
    name = record.get("name")
    if not isinstance(name, str):
        raise ValueError("a span record names its span")
    return name


def run_metric_error(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    config.enable_metrics = True
    factory_for()
    litellm.callback_settings = {"otel": {"attributes": input_value["filter"]}}
    meter_provider = SdkMeterProvider()
    logger, _ = build_logger(config, None, meter_provider)
    counter = ErrorCounter()
    root = logging.getLogger("LiteLLM")
    root.addHandler(counter)
    metadata = payload_metadata(None, None)
    payload = payload_document("completion", "success", metadata)
    kwargs = close_kwargs(payload, metadata, None)
    try:
        asyncio.run(log_event(logger, kwargs, False))
        asyncio.run(log_event(logger, kwargs, False))
    finally:
        root.removeHandler(counter)
    return {"error_logs": counter.count}


def run_close_dedup(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory_for()
    logger, default_exporter = build_logger(config, input_value["callback_name"])
    anchor(logger.tracer_provider)
    logger.log_pre_api_call(MODEL, [], pre_call_details(None, None))
    metadata = payload_metadata(None, None)
    success = payload_document("completion", "success", metadata)
    asyncio.run(log_event(logger, close_kwargs(success, metadata, None), False))
    failure = payload_document("completion", "failure", metadata)
    asyncio.run(log_event(logger, close_kwargs(failure, metadata, None), True))
    names = sorted(record_name(record) for record in span_records([default_exporter]))
    return {"names": names, "count": len(names)}


def run_cache_eviction(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory = factory_for()
    cache = TenantTracerCache(config, input_value["callback_name"], "litellm")
    provider, _ = default_provider(config)
    default = provider.get_tracer("litellm")
    held: TracerProvider | None = None
    for index, space in enumerate(input_value["spaces"]):
        route = cache.route_for(default, {"arize_space_id": space}, None)
        if index == 0:
            held = route.provider
        else:
            cache.release(route.provider)
    builds_before_release = len(factory.specs)
    cache.release(held)
    shutdowns = sum(exporter.shutdowns for exporter in factory.exporters)
    last = input_value["spaces"][-1]
    cache.route_for(default, {"arize_space_id": last}, None)
    return {
        "builds": builds_before_release,
        "shutdowns_after_release": shutdowns,
        "builds_after_reuse": len(factory.specs),
    }


def run_project_headers(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory = factory_for()
    cache = TenantTracerCache(config, input_value["callback_name"], "litellm")
    provider, _ = default_provider(config)
    auth_metadata: Params | None = None
    project_name = input_value["project_name"]
    if project_name is not None:
        auth_metadata = {"phoenix_project_name": project_name}
    route = cache.route_for(
        provider.get_tracer("litellm"), input_value["credentials"], auth_metadata
    )
    if not factory.specs:
        raise ValueError("the project route built no provider")
    spec = factory.specs[0]
    pairs = (spec.headers or "").split(",")
    return {
        "builds": len(factory.specs),
        "header_pairs": sorted(pairs),
        "endpoint": spec.endpoint,
        "detached": route.detached,
    }


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "pre_call":
        return run_pre_call(input_value)
    if function == "mcp_tool_call":
        return run_mcp_tool_call(input_value)
    if function == "metric_error":
        return run_metric_error(input_value)
    if function == "close_dedup":
        return run_close_dedup(input_value)
    if function == "cache_eviction":
        return run_cache_eviction(input_value)
    return run_project_headers(input_value)
