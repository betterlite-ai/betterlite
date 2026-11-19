from typing import Literal, TypeAlias, TypedDict, cast

from litellm._logging import redact_internal_details_from_client_message
from litellm.anthropic_interface.exceptions.exception_mapping_utils import anthropic_error_sse_frame
from litellm.proxy._types import ProxyException
from litellm.proxy.common_utils.openai_error_payload import (
    error_status_code,
    openai_error_param,
    openai_error_type,
)
from litellm.proxy.common_utils.sse_keepalive import (
    SSE_STREAM_START_TAIL,
    advance_sse_tail,
    seal_open_sse_frame,
)
from litellm.proxy.proxy_server import (
    _should_return_raw_model_name,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L279
)
from pydantic import JsonValue


class SealInput(TypedDict):
    kind: Literal["seal"]
    writes: list[str]


class RawModelInput(TypedDict):
    kind: Literal["raw_model"]
    request_data: dict[str, JsonValue]


class FailureSpec(TypedDict):
    message: str
    type: str | None
    param: str | None
    status_code: int | None


class FailureInput(TypedDict):
    kind: Literal["failure"]
    failure: FailureSpec


Input: TypeAlias = SealInput | RawModelInput | FailureInput


class FailureError(Exception):
    message: str
    type: str | None
    param: str | None
    status_code: int | None

    def __init__(self, spec: FailureSpec) -> None:
        super().__init__(spec["message"])
        self.message = spec["message"]
        self.type = spec["type"]
        self.param = spec["param"]
        self.status_code = spec["status_code"]


def seal_value(call: SealInput) -> dict[str, JsonValue]:
    tail = SSE_STREAM_START_TAIL
    tails: list[JsonValue] = [tail.hex()]
    seals: list[JsonValue] = [seal_open_sse_frame(tail)]
    for write in call["writes"]:
        tail = advance_sse_tail(tail, write)
        tails.append(tail.hex())
        seals.append(seal_open_sse_frame(tail))
    return {"tails": tails, "seals": seals}


def raw_model_value(call: RawModelInput) -> dict[str, JsonValue]:
    request_data = cast("dict[str, object]", call["request_data"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L279
    return {"raw": _should_return_raw_model_name(request_data)}


def error_body(exception: ProxyException) -> dict[str, JsonValue]:
    return {
        "message": exception.message,
        "type": exception.type,
        "param": exception.param,
        "code": exception.code,
    }


def failure_value(call: FailureInput) -> dict[str, JsonValue]:
    error = FailureError(call["failure"])
    status = error_status_code(error, 500)
    exception = ProxyException(
        message=redact_internal_details_from_client_message(error.message),
        type=openai_error_type(error, status),
        param=openai_error_param(error),
        code=status,
    )
    frame = anthropic_error_sse_frame(
        status_code=error_status_code(exception, 500), raw_message=exception.message
    )
    return {
        "error": error_body(exception),
        "frame_status": frame.status_code,
        "frame_body": str(frame),
    }


def run(input_value: Input) -> JsonValue:
    if input_value["kind"] == "seal":
        return seal_value(input_value)
    if input_value["kind"] == "raw_model":
        return raw_model_value(input_value)
    return failure_value(input_value)
