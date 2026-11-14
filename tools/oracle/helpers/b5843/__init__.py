import datetime as dt
from typing import Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.oci.chat.transformation import (
    OCIChatConfig,
    OCIStreamWrapper,
    get_vendor_from_model,
)
from litellm.types.utils import ModelResponse
from openai import APIStatusError
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

API_BASE = "https://example.test"
DATA_PREFIX = "data:"
STREAM_MODEL = "meta.llama"

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ExchangeCall(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    params: dict[str, JsonValue]
    reply: str
    status: int
    drop_params: bool


class ResponseCall(TypedDict):
    model: str
    status: int
    body: JsonValue


class StreamDelta(TypedDict, total=False):
    content: str | None
    tool_calls: list[dict[str, JsonValue]] | None


class StreamChoice(TypedDict):
    delta: StreamDelta
    finish_reason: NotRequired[str | None]


class StreamDump(TypedDict):
    choices: list[StreamChoice]


class MessageDump(TypedDict, total=False):
    content: str | None


class ResponseChoiceDump(TypedDict):
    message: MessageDump
    finish_reason: NotRequired[str | None]


class ResponseDump(TypedDict):
    model: str
    choices: list[ResponseChoiceDump]


STREAM_DUMP: TypeAdapter[StreamDump] = TypeAdapter(StreamDump)
RESPONSE_DUMP: TypeAdapter[ResponseDump] = TypeAdapter(ResponseDump)


class ModelsCall(TypedDict):
    function: Literal["supported", "vendor"]
    models: list[str]


class ExchangeInput(TypedDict):
    function: Literal["exchange"]
    credentials: dict[str, JsonValue]
    replies: dict[str, JsonValue]
    exchanges: list[ExchangeCall]


class ResponseInput(TypedDict):
    function: Literal["response"]
    credentials: dict[str, JsonValue]
    responses: list[ResponseCall]


class StreamInput(TypedDict):
    function: Literal["stream"]
    reads: list[str]


Call: TypeAlias = ModelsCall | ExchangeInput | ResponseInput | StreamInput


class Recorder:
    def __init__(self, status: int, reply: JsonValue) -> None:
        self.status = status
        self.reply = reply
        self.bodies: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.bodies.append(JSON.validate_json(request.content))
        return httpx.Response(self.status, json=self.reply, request=request)


def supported_params(models: list[str]) -> JsonValue:
    config = OCIChatConfig()
    return {
        "models": [
            {
                "model": model,
                "params": JSON.validate_python(config.get_supported_openai_params(model=model)),
            }
            for model in models
        ]
    }


def exchange_call(
    credentials: dict[str, JsonValue],
    replies: dict[str, JsonValue],
    call: ExchangeCall,
) -> JsonValue:
    recorder = Recorder(call["status"], replies[call["reply"]])
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder)))
    params = {**credentials, **call["params"]}
    try:
        litellm.completion(
            model=call["model"],
            messages=call["messages"],
            api_base=API_BASE,
            custom_llm_provider="oci",
            client=client,
            drop_params=call["drop_params"],
            **params,
        )
    except APIStatusError as error:
        return {"status": error.status_code}
    return {"body": recorder.bodies[-1]}


def response_result(credentials: dict[str, JsonValue], call: ResponseCall) -> JsonValue:
    recorder = Recorder(call["status"], call["body"])
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder)))
    try:
        response = litellm.completion(
            model=call["model"],
            messages=[{"role": "user", "content": "hi"}],
            api_base=API_BASE,
            custom_llm_provider="oci",
            client=client,
            **credentials,
        )
    except APIStatusError as error:
        return {"status": error.status_code}
    match response:
        case ModelResponse():
            dumped = RESPONSE_DUMP.validate_json(response.model_dump_json())
            choice = dumped["choices"][0]
            return {
                "model": dumped["model"],
                "content": choice["message"].get("content"),
                "finish_reason": choice.get("finish_reason"),
            }
        case CustomStreamWrapper():
            raise ValueError("the OCI chat reply was not a plain response")


def stream_items(reads: list[str]) -> list[JsonValue]:
    wrapper = OCIStreamWrapper(
        completion_stream=iter(()),
        model=STREAM_MODEL,
        logging_obj=Logging(
            model=STREAM_MODEL,
            messages=[],
            stream=True,
            call_type="completion",
            start_time=dt.datetime.now(tz=dt.UTC),
            litellm_call_id="oci-chat-stream",
            function_id="oci-chat-stream",
        ),
        custom_llm_provider="oci",
    )
    items: list[JsonValue] = []
    for line in "".join(reads).split("\n"):
        stripped = line.strip()
        if not stripped.startswith(DATA_PREFIX):
            continue
        chunk = wrapper.chunk_creator(stripped)
        if chunk is None:
            items.append({"done": True})
            continue
        dumped = STREAM_DUMP.validate_json(chunk.model_dump_json())
        choice = dumped["choices"][0]
        items.append(
            {
                "content": choice["delta"].get("content"),
                "tool_calls": choice["delta"].get("tool_calls") is not None,
                "finish_reason": choice.get("finish_reason"),
            }
        )
    return items


def run(input_value: Call) -> JsonValue:
    match input_value["function"]:
        case "supported":
            return supported_params(input_value["models"])
        case "vendor":
            return {
                "models": [
                    {"model": model, "vendor": get_vendor_from_model(model).name}
                    for model in input_value["models"]
                ]
            }
        case "exchange":
            return {
                "calls": [
                    exchange_call(input_value["credentials"], input_value["replies"], call)
                    for call in input_value["exchanges"]
                ]
            }
        case "response":
            return {
                "calls": [
                    response_result(input_value["credentials"], call)
                    for call in input_value["responses"]
                ]
            }
        case "stream":
            return {"items": stream_items(input_value["reads"])}
