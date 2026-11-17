import asyncio
import json
from collections.abc import Coroutine
from typing import Final, Literal, Protocol, TypedDict

import litellm
from litellm._logging import json_logs, log_level, resolve_log_level, set_trace_id, trace_id_var
from litellm._service_logger import ServiceLogging
from litellm.integrations.prometheus_services import PrometheusServicesLogger
from litellm.types.services import ServiceLoggerPayload, ServiceTypes
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class CorrelationInput(TypedDict):
    op: Literal["correlation_sanitize"]
    call_id: str


class LogLevelInput(TypedDict):
    op: Literal["log_level"]
    environment: dict[str, str]


class SuccessInput(TypedDict):
    op: Literal["service_success"]
    service: str
    call_type: str
    duration: float
    caller: str | None


class FailureInput(TypedDict):
    op: Literal["service_failure"]
    service: str
    call_type: str
    duration: float
    caller: str | None
    error: str


Input = CorrelationInput | LogLevelInput | SuccessInput | FailureInput


class PayloadRecord(TypedDict):
    is_error: bool
    error: str | None
    service: str
    duration: float
    call_type: str
    caller: str | None
    event_metadata: dict[str, JsonValue] | None


# the two hook entry points this helper drives, typed so the untyped LiteLLM logger stays out
class ServiceDispatch(Protocol):
    def async_service_success_hook(
        self,
        *,
        service: ServiceTypes,
        call_type: str,
        duration: float,
        caller: str | None,
    ) -> Coroutine[None, None, None]: ...

    def async_service_failure_hook(
        self,
        *,
        service: ServiceTypes,
        duration: float,
        error: str,
        call_type: str,
        caller: str | None,
    ) -> Coroutine[None, None, None]: ...


PAYLOAD: Final[TypeAdapter[PayloadRecord]] = TypeAdapter(PayloadRecord)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


# the recorder stands in for the Prometheus service, so the case compares the payload LiteLLM built
class PayloadRecorder(PrometheusServicesLogger):
    def __init__(self) -> None:
        self.payloads: list[PayloadRecord] = []

    @override
    async def async_service_success_hook(self, payload: ServiceLoggerPayload) -> None:
        self.payloads.append(PAYLOAD.validate_json(payload.model_dump_json()))

    @override
    async def async_service_failure_hook(
        self, payload: ServiceLoggerPayload, error: str | Exception
    ) -> None:
        _ = error
        self.payloads.append(PAYLOAD.validate_json(payload.model_dump_json()))


def recorder() -> tuple[ServiceLogging, PayloadRecorder]:
    litellm.service_callback = []
    service_logger = ServiceLogging()
    payloads = PayloadRecorder()
    service_logger.prometheusServicesLogger = payloads
    litellm.service_callback = ["prometheus_system"]
    return service_logger, payloads


def payload_output(payloads: PayloadRecorder) -> JsonValue:
    records: list[JsonValue] = [
        JSON.validate_json(json.dumps(record)) for record in payloads.payloads
    ]
    value: dict[str, JsonValue] = {"payloads": records}
    return value


def correlation_value(input_value: CorrelationInput) -> JsonValue:
    set_trace_id(input_value["call_id"])
    value: dict[str, JsonValue] = {"sanitized": trace_id_var.get()}
    return value


def level_value() -> JsonValue:
    value: dict[str, JsonValue] = {
        "numeric": resolve_log_level(log_level),
        "json_logs": json_logs,
    }
    return value


# one body for both hooks, so the two payload shapes differ only in the call they make
def payload_value(input_value: SuccessInput | FailureInput) -> JsonValue:
    service_logger, payloads = recorder()
    dispatch: ServiceDispatch = service_logger
    if input_value["op"] == "service_success":
        asyncio.run(
            dispatch.async_service_success_hook(
                service=ServiceTypes(input_value["service"]),
                call_type=input_value["call_type"],
                duration=input_value["duration"],
                caller=input_value["caller"],
            )
        )
    else:
        asyncio.run(
            dispatch.async_service_failure_hook(
                service=ServiceTypes(input_value["service"]),
                duration=input_value["duration"],
                error=input_value["error"],
                call_type=input_value["call_type"],
                caller=input_value["caller"],
            )
        )
    return payload_output(payloads)


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "correlation_sanitize":
        return correlation_value(input_value)
    if input_value["op"] == "log_level":
        return level_value()
    return payload_value(input_value)
