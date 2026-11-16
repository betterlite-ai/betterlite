import asyncio
import os
from collections.abc import Awaitable
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.compresr.compresr import CompresrGuardrail
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class Config(TypedDict, total=False):
    api_base: str
    api_key: str
    min_chars_to_compress: int
    enable_retrieval: bool
    allow_bypass_header: bool


class Reply(TypedDict):
    status: int
    body: str


class Caller(TypedDict):
    scope: str
    call_id: str


class InitCall(TypedDict):
    function: Literal["init"]
    config: Config
    env: dict[str, str]


class ApplyCall(TypedDict):
    function: Literal["apply"]
    config: Config
    reply: Reply
    inputs: dict[str, JsonValue]
    headers: dict[str, str] | None
    caller: Caller | None


# LiteLLM keeps live objects in these details, so they carry more than JsonValue allows
Details: TypeAlias = dict[str, object]


class LoggingDetails(Protocol):
    model_call_details: Details


@runtime_checkable
class Applier(Protocol):
    def apply_guardrail(
        self, inputs: dict[str, JsonValue], request_data: dict[str, JsonValue],
        input_type: Literal["request"], logging_obj: Logging | None = None,
    ) -> Awaitable[dict[str, JsonValue]]: ...


class ApplierRecord(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    applier: Applier


def logging_for(caller: Caller) -> Logging:
    logging_obj = Logging(
        model="gpt-4o",
        messages=None,
        stream=False,
        call_type="acompletion",
        start_time=datetime(2024, 1, 1, tzinfo=UTC),
        litellm_call_id=caller["call_id"],
        function_id="b3765",
    )
    details: LoggingDetails = logging_obj
    auth = UserAPIKeyAuth(api_key=caller["scope"])
    params: Details = {"metadata": {"user_api_key_auth": auth}}
    details.model_call_details.update({"litellm_params": params})
    return logging_obj


def init(call: InitCall) -> JsonValue:
    # an empty variable reads as unset, so a host variable never leaks into the case
    environment = {"COMPRESR_API_KEY": "", "COMPRESR_API_BASE": "", **call["env"]}
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(environment)
    try:
        guardrail = CompresrGuardrail(**call["config"])
    except ValueError:
        return {"accepted": False}
    finally:
        os.environ.clear()
        os.environ.update(previous)
    return {
        "accepted": True,
        "api_base": guardrail.compresr_api_base,
        "model": guardrail.compression_model,
        "ratio": guardrail.target_compression_ratio,
        "coarse": guardrail.coarse,
        "dynamic": guardrail.dynamic,
        "min_chars": guardrail.min_chars_to_compress,
        "tool_outputs": guardrail.compress_tool_outputs,
        "retrieval": guardrail.enable_retrieval,
        "max_bytes": guardrail.max_bytes_per_call,
        "bypass": guardrail.allow_bypass_header,
    }


class Recorder:
    def __init__(self, reply: Reply) -> None:
        self.requests: list[JsonValue] = []
        self.reply = reply

    def handle(self, request: httpx.Request) -> httpx.Response:
        sent = dict(request.headers.items())
        api_key = sent.get("x-api-key")
        self.requests.append(
            {
                "path": request.url.path,
                "api_key": "" if api_key is None else api_key,
                "body": JSON.validate_json(request.content),
            }
        )
        return httpx.Response(self.reply["status"], content=self.reply["body"].encode())


async def apply_once(
    applier: Applier, call: ApplyCall, request_data: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    caller = call["caller"]
    logging_obj = None if caller is None else logging_for(caller)
    return await applier.apply_guardrail(
        call["inputs"], request_data, "request", logging_obj
    )


def apply(call: ApplyCall) -> JsonValue:
    guardrail = CompresrGuardrail(**call["config"])
    recorder = Recorder(call["reply"])
    transport = httpx.MockTransport(recorder.handle)
    guardrail.async_handler.client = httpx.AsyncClient(transport=transport)
    headers = call["headers"]
    request_data: dict[str, JsonValue] = {}
    if headers is not None:
        request_data["proxy_server_request"] = {"headers": dict(headers)}
    # LiteLLM types apply_guardrail with GenericGuardrailAPIInputs, whose message TypedDicts carry Any
    applier = ApplierRecord.model_validate({"applier": guardrail}).applier
    output = asyncio.run(apply_once(applier, call, request_data))
    return {"inputs": output, "requests": recorder.requests}


def run(input_value: InitCall | ApplyCall) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "init":
            answer = init(input_value)
        case "apply":
            answer = apply(input_value)
    return answer
