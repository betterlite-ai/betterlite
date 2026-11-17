import asyncio
import logging
from typing import cast

import litellm
from litellm.integrations.otel.plumbing.routing import TenantTracerCache
from opentelemetry.sdk.metrics import MeterProvider as SdkMeterProvider
from opentelemetry.sdk.trace import TracerProvider
from pydantic import JsonValue as Json
from typing_extensions import override

from .exporters import (
    anchor,
    build_logger,
    config_from,
    default_provider,
    factory_for,
    span_records,
)
from .payloads import close_kwargs, log_event, payload_document, payload_metadata, pre_call_details
from .wire import JSON, MESSAGES, MODEL, Input, Params


class ErrorCounter(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.ERROR)
        self.count = 0

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno >= logging.ERROR:
            self.count += 1


def run_pre_call(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory = factory_for()
    logger, default_exporter = build_logger(config, input_value["callback_name"])
    if input_value["anchored"]:
        anchor(logger.tracer_provider)
    logger.log_pre_api_call(  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/logger.py#L277
        MODEL,
        MESSAGES,
        pre_call_details(input_value["credentials"], input_value["auth_metadata"]),
    )
    builds_after_pre_call = len(factory.specs)
    metadata = payload_metadata(input_value["auth_metadata"], None)
    payload = payload_document("completion", "success", metadata)
    kwargs = close_kwargs(payload, metadata, input_value["credentials"])
    asyncio.run(log_event(logger, kwargs, failed=False))
    return {
        "builds_after_pre_call": builds_after_pre_call,
        "builds_after_close": len(factory.specs),
        "routed_spans": JSON.validate_python(span_records(factory.exporters)),
        "default_spans": JSON.validate_python(span_records([default_exporter])),
    }


def run_mcp_tool_call(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory = factory_for()
    logger, default_exporter = build_logger(config, input_value["callback_name"])
    metadata = payload_metadata(input_value["auth_metadata"], "search")
    payload = payload_document("call_mcp_tool", "success", metadata)
    kwargs = close_kwargs(payload, metadata, input_value["credentials"])
    asyncio.run(log_event(logger, kwargs, failed=False))
    records = span_records([default_exporter])
    names = sorted(record["name"] for record in records)
    return {
        "names": JSON.validate_python(names),
        "count": len(records),
        "builds": len(factory.specs),
    }


def run_metric_error(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    config.enable_metrics = True
    factory_for()
    callback_settings = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L184
        "dict[str, dict[str, Json]]", litellm.callback_settings
    )
    callback_settings["otel"] = {"attributes": input_value["filter"]}
    meter_provider = SdkMeterProvider()
    logger, _ = build_logger(config, None, meter_provider)
    counter = ErrorCounter()
    root = logging.getLogger("LiteLLM")
    root.addHandler(counter)
    metadata = payload_metadata(None, None)
    payload = payload_document("completion", "success", metadata)
    kwargs = close_kwargs(payload, metadata, None)
    try:
        asyncio.run(log_event(logger, kwargs, failed=False))
        asyncio.run(log_event(logger, kwargs, failed=False))
    finally:
        root.removeHandler(counter)
    return {"error_logs": counter.count}


def run_close_dedup(input_value: Input) -> Json:
    config = config_from(input_value["exporters"])
    factory_for()
    logger, default_exporter = build_logger(config, input_value["callback_name"])
    anchor(logger.tracer_provider)
    logger.log_pre_api_call(  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/logger.py#L277
        MODEL,
        MESSAGES,
        pre_call_details(None, None),
    )
    metadata = payload_metadata(None, None)
    success = payload_document("completion", "success", metadata)
    asyncio.run(log_event(logger, close_kwargs(success, metadata, None), failed=False))
    failure = payload_document("completion", "failure", metadata)
    asyncio.run(log_event(logger, close_kwargs(failure, metadata, None), failed=True))
    records = span_records([default_exporter])
    names = sorted(record["name"] for record in records)
    return {
        "names": JSON.validate_python(names),
        "count": len(records),
    }


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
        provider.get_tracer("litellm"),
        input_value["credentials"],  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/otel/plumbing/routing.py#L216
        auth_metadata,
    )
    if not factory.specs:
        raise ValueError("the project route built no provider")
    spec = factory.specs[0]
    header_text = spec.headers
    pairs = header_text.split(",") if header_text is not None else []
    header_pairs = sorted(pairs)
    return {
        "builds": len(factory.specs),
        "header_pairs": JSON.validate_python(header_pairs),
        "endpoint": spec.endpoint,
        "detached": route.detached,
    }
