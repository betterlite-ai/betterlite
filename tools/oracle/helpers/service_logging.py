import asyncio
import json
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm._logging import json_logs, log_level, resolve_log_level, set_trace_id, trace_id_var
from litellm._service_logger import ServiceLogging
from litellm.integrations.prometheus_services import PrometheusServicesLogger
from litellm.types.services import ServiceLoggerPayload, ServiceTypes
from pydantic import JsonValue, TypeAdapter

Operation = Literal[
    "correlation_sanitize", "log_level", "service_success", "service_failure"
]


class Input(TypedDict):
    op: Operation
    call_id: NotRequired[str]
    service: NotRequired[str]
    call_type: NotRequired[str]
    duration: NotRequired[float]
    caller: NotRequired[str | None]
    error: NotRequired[str]


class PayloadRecord(TypedDict):
    is_error: bool
    error: str | None
    service: str
    duration: float
    call_type: str
    caller: str | None
    event_metadata: dict[str, JsonValue] | None


PAYLOAD: Final[TypeAdapter[PayloadRecord]] = TypeAdapter(PayloadRecord)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


# the recorder stands in for the Prometheus service, so the case compares the payload LiteLLM built
class PayloadRecorder(PrometheusServicesLogger):
    def __init__(self) -> None:
        self.payloads: list[PayloadRecord] = []

    async def async_service_success_hook(self, payload: ServiceLoggerPayload) -> None:
        self.payloads.append(PAYLOAD.validate_json(payload.model_dump_json()))

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


def correlation_value(input_value: Input) -> JsonValue:
    set_trace_id(input_value["call_id"])
    value: dict[str, JsonValue] = {"sanitized": trace_id_var.get()}
    return value


def level_value() -> JsonValue:
    value: dict[str, JsonValue] = {
        "numeric": resolve_log_level(log_level),
        "json_logs": json_logs,
    }
    return value


def success_value(input_value: Input) -> JsonValue:
    service_logger, payloads = recorder()
    asyncio.run(
        service_logger.async_service_success_hook(
            service=ServiceTypes(input_value["service"]),
            call_type=input_value["call_type"],
            duration=input_value["duration"],
            caller=input_value["caller"],
        )
    )
    return payload_output(payloads)


def failure_value(input_value: Input) -> JsonValue:
    service_logger, payloads = recorder()
    asyncio.run(
        service_logger.async_service_failure_hook(
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
    if input_value["op"] == "service_success":
        return success_value(input_value)
    return failure_value(input_value)
