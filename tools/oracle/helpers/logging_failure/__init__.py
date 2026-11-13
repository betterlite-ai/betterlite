from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING, cast
from unittest.mock import patch

import litellm
from litellm import ModelResponse
from litellm._logging import session_id_var, trace_id_var
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import Usage

from .fixtures import (
    Cost,
    Executor,
    Json,
    Probe,
    Recorders,
    Report,
    build_logging,
    build_report,
    float_value,
    frozen_datetime,
    list_value,
    micros,
    object_value,
    optional_float,
    payload_builder,
    payload_record,
    snapshot_context,
    snapshot_details,
    string_value,
)

if TYPE_CHECKING:
    import datetime as dt

# pyright: reportPrivateUsage=false, reportUnknownMemberType=false


def run(input_value: dict[str, Json]) -> Json:
    instances = input_value.get("instances")
    if not isinstance(instances, list):
        raise ValueError("instances must be a list")
    results = [_instance(cast("dict[str, Json]", value)) for value in instances]
    return cast(Json, {"instances": results})


def _instance(value: dict[str, Json]) -> Json:
    init = object_value(value.get("init"))
    correlation = object_value(value.get("correlation"))
    settings = object_value(correlation.get("settings"))
    context = object_value(correlation.get("context"))
    operations = value.get("operations")
    if not isinstance(operations, list):
        raise ValueError("operations must be a list")
    trace_id_var.set(string_value(context.get("trace_id")))
    session_id_var.set(string_value(context.get("session_id")))
    previous_flag = litellm.request_correlation_in_logs
    previous_disable = litellm.global_disable_no_log_param
    previous_redact = litellm.turn_off_message_logging
    litellm.request_correlation_in_logs = settings.get("request_correlation_in_logs") is True
    litellm.global_disable_no_log_param = (
        object_value(init.get("callback_policy")).get("global_disable_no_log_param") is True
    )
    litellm.turn_off_message_logging = False
    try:
        return asyncio.run(_run_instance(init, settings, operations, value))
    finally:
        litellm.request_correlation_in_logs = previous_flag
        litellm.global_disable_no_log_param = previous_disable
        litellm.turn_off_message_logging = previous_redact
        trace_id_var.set("")
        session_id_var.set("")


async def _run_instance(
    init: dict[str, Json],
    settings: dict[str, Json],
    operations: list[Json],
    value: dict[str, Json],
) -> Json:
    recorders = Recorders()
    logging_obj = build_logging(init, settings)
    specs = list_value(value.get("loggers"))
    probes: list[Probe] = []
    for spec in specs:
        fields = object_value(spec)
        named = cast("type[Probe]", type(string_value(fields.get("name")), (Probe,), {}))
        probes.append(named(fields, recorders.calls))
    for probe in probes:
        probe.stream_flag = cast("bool | None", logging_obj.stream) is True
    litellm.callbacks = []
    litellm.success_callback = []
    litellm._async_success_callback = []  # noqa: SLF001
    litellm.failure_callback = probes
    litellm._async_failure_callback = probes  # noqa: SLF001
    report = build_report(object_value(value.get("failure")), init)
    raw_payload = value.get("payload")
    payload = None if raw_payload is None else payload_record(object_value(raw_payload))
    with (
        patch.object(
            litellm_logging, "get_standard_logging_object_payload", payload_builder(payload)
        ),
        patch.object(Logging, "_response_cost_calculator", Cost(recorders)),
        patch.object(litellm_logging, "executor", Executor(recorders.jobs)),
    ):
        steps = [
            await _step(logging_obj, report, object_value(operation), recorders)
            for operation in operations
        ]
    return cast(Json, {"steps": steps})


async def _step(
    logging_obj: Logging,
    report: Report,
    operation: dict[str, Json],
    recorders: Recorders,
) -> Json:
    recorders.cost = optional_float(operation.get("cost"))
    before = len(recorders.calls)
    timing = await _apply(logging_obj, report, operation)
    return cast(
        Json,
        {
            "op": string_value(operation.get("op")),
            "details": snapshot_details(logging_obj),
            "context": snapshot_context(),
            "calls": cast(Json, recorders.calls[before:]),
            "executor_jobs": len(recorders.jobs),
            "timing": timing,
        },
    )


async def _apply(logging_obj: Logging, report: Report, operation: dict[str, Json]) -> Json:
    name = string_value(operation.get("op"))
    if name == "record_partial_usage":
        usage = Usage.model_validate(object_value(operation.get("usage")))
        logging_obj.record_partial_usage_for_failure(usage, float_value(operation.get("cost")))
        return None
    if name == "record_assembled_response":
        raw_usage = operation.get("usage")
        usage = None if raw_usage is None else Usage.model_validate(object_value(raw_usage))
        logging_obj.record_assembled_response_for_failure(ModelResponse(usage=usage))
        return None
    if name == "failure_handler_helper_fn":
        start, end = cast(
            "tuple[dt.datetime, dt.datetime]",
            logging_obj._failure_handler_helper_fn(  # noqa: SLF001
                report.error, report.traceback, report.start_time, report.end_time
            ),
        )
        return cast(Json, {"start_micros": micros(start), "end_micros": micros(end)})
    if name == "failure_handler":
        logging_obj.failure_handler(
            report.error, report.traceback, report.start_time, report.end_time
        )
        return None
    if name == "async_failure_handler":
        await logging_obj.async_failure_handler(
            report.error, report.traceback, report.start_time, report.end_time
        )
        return None
    if name == "dispatch_failure_handlers":
        with patch.object(litellm_logging, "datetime", frozen_datetime(logging_obj.start_time)):
            await logging_obj.dispatch_failure_handlers(
                report.error,
                report.traceback,
                prefer_async_handlers=operation.get("prefer_async_handlers") is True,
            )
        return None
    raise ValueError(f"unknown logging_failure operation {name}")
