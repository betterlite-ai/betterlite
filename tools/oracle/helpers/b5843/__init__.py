import datetime
from typing import Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.oci.chat.cohere import handle_cohere_response
from litellm.llms.oci.chat.generic import handle_generic_response
from litellm.llms.oci.chat.transformation import (
    OCIChatConfig,
    OCIStreamWrapper,
    get_vendor_from_model,
)
from litellm.llms.oci.common_utils import OCIError
from litellm.types.llms.oci import OCIVendors
from litellm.types.utils import ModelResponse, ModelResponseStream
from openai import APIStatusError
from pydantic import JsonValue, TypeAdapter, ValidationError

API_BASE = "https://example.test"
DATA_PREFIX = "data:"
INVALID_RESPONSE_FORMAT = "Invalid response format from OCI"
STREAM_MODEL = "meta.llama"

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


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


class ToolCallDump(TypedDict, total=False):
    id: str | None
    function: dict[str, JsonValue] | None


class MessageDump(TypedDict, total=False):
    content: str | None
    tool_calls: list[ToolCallDump] | None


class ResponseChoiceDump(TypedDict):
    message: MessageDump
    finish_reason: NotRequired[str | None]


class UsageDump(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseDump(TypedDict):
    model: str
    choices: list[ResponseChoiceDump]
    usage: UsageDump


STREAM_DUMP: TypeAdapter[StreamDump] = TypeAdapter(StreamDump)
RESPONSE_DUMP: TypeAdapter[ResponseDump] = TypeAdapter(ResponseDump)


class SupportedCall(TypedDict):
    function: Literal["supported"]
    models: list[str]


class VendorCall(TypedDict):
    function: Literal["vendor"]
    models: list[str]


class ExchangeInput(TypedDict):
    function: Literal["exchange"]
    credentials: dict[str, JsonValue]
    replies: dict[str, JsonValue]
    exchanges: list[ExchangeCall]


class ResponseInput(TypedDict):
    function: Literal["response"]
    responses: list[ResponseCall]


class StreamInput(TypedDict):
    function: Literal["stream"]
    reads: list[str]


Call: TypeAlias = SupportedCall | VendorCall | ExchangeInput | ResponseInput | StreamInput


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
    found: list[JsonValue] = []
    for model in models:
        params = config.get_supported_openai_params(model=model)
        found.append({"model": model, "params": JSON.validate_python(params)})
    return {"models": found}


def vendor_of(models: list[str]) -> JsonValue:
    found: list[JsonValue] = []
    for model in models:
        vendor = get_vendor_from_model(model).name
        found.append({"model": model, "vendor": vendor})
    return {"models": found}


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


def project_tool_call(call: ToolCallDump) -> JsonValue:
    function = call.get("function")
    name: JsonValue = None if function is None else function.get("name")
    return {"id": call.get("id"), "name": name}


def project_response(response: ModelResponse) -> JsonValue:
    dumped = RESPONSE_DUMP.validate_json(response.model_dump_json())
    choice = dumped["choices"][0]
    message = choice["message"]
    calls = message.get("tool_calls")
    tool_calls: JsonValue = None
    if calls is not None:
        tool_calls = [project_tool_call(call) for call in calls]
    usage = dumped["usage"]
    return {
        "model": dumped["model"],
        "content": message.get("content"),
        "finish_reason": choice.get("finish_reason"),
        "tool_calls": tool_calls,
        "usage": {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
        },
    }


def response_projection(call: ResponseCall) -> JsonValue:
    raw = httpx.Response(call["status"], json=call["body"])
    try:
        document = OBJECT.validate_python(call["body"])
    except ValidationError as error:
        raise OCIError(call["status"], INVALID_RESPONSE_FORMAT) from error
    problem = document.get("error")
    if problem is not None:
        raise OCIError(call["status"], str(problem))
    response = ModelResponse()
    if get_vendor_from_model(call["model"]) == OCIVendors.COHERE:
        return project_response(handle_cohere_response(document, call["model"], response, raw))
    return project_response(handle_generic_response(document, call["model"], response, raw))


def response_result(call: ResponseCall) -> JsonValue:
    try:
        return response_projection(call)
    except BaseLLMException as error:
        return {"status": error.status_code}


def sse_data_lines(reads: list[str]) -> list[str]:
    lines: list[str] = []
    buffer = ""
    for read in reads:
        buffer += read
        while "\n" in buffer:
            line, buffer = buffer.split("\n", 1)
            stripped = line.strip()
            if stripped.startswith(DATA_PREFIX):
                lines.append(stripped)
    tail = buffer.strip()
    if tail.startswith(DATA_PREFIX):
        lines.append(tail)
    return lines


def stream_logger() -> Logging:
    return Logging(
        model=STREAM_MODEL,
        messages=[],
        stream=True,
        call_type="completion",
        start_time=datetime.datetime.now(tz=datetime.UTC),
        litellm_call_id="oci-chat-stream",
        function_id="oci-chat-stream",
    )


def project_chunk(chunk: ModelResponseStream) -> JsonValue:
    dumped = STREAM_DUMP.validate_json(chunk.model_dump_json())
    choice = dumped["choices"][0]
    delta = choice["delta"]
    return {
        "content": delta.get("content"),
        "tool_calls": delta.get("tool_calls") is not None,
        "finish_reason": choice.get("finish_reason"),
    }


def stream_items(reads: list[str]) -> list[JsonValue]:
    wrapper = OCIStreamWrapper(
        completion_stream=iter(()),
        model=STREAM_MODEL,
        logging_obj=stream_logger(),
        custom_llm_provider="oci",
    )
    items: list[JsonValue] = []
    for line in sse_data_lines(reads):
        chunk = wrapper.chunk_creator(line)
        if chunk is None:
            items.append({"done": True})
        else:
            items.append(project_chunk(chunk))
    return items


def run(input_value: Call) -> JsonValue:
    match input_value["function"]:
        case "supported":
            return supported_params(input_value["models"])
        case "vendor":
            return vendor_of(input_value["models"])
        case "exchange":
            return {
                "calls": [
                    exchange_call(input_value["credentials"], input_value["replies"], call)
                    for call in input_value["exchanges"]
                ]
            }
        case "response":
            return {"calls": [response_result(call) for call in input_value["responses"]]}
        case "stream":
            return {"items": stream_items(input_value["reads"])}
