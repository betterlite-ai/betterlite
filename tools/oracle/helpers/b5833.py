import os
from collections.abc import Callable
from typing import Final, Literal, NotRequired, Protocol, TypedDict
from unittest.mock import MagicMock

import httpx
import litellm
from litellm import ModelResponse, ModelResponseStream, Usage
from litellm.exceptions import BadRequestError, UnsupportedParamsError
from litellm.llms.groq.chat.transformation import (
    GroqChatCompletionStreamingHandler as GroqStreamHandler,
)
from litellm.llms.groq.chat.transformation import GroqChatConfig
from litellm.llms.groq.cost_calculator import cost_per_web_search_request
from litellm.llms.groq.stt.transformation import GroqSTTConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.utils import ServerToolUse
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

Object = dict[str, Json]

Answer = Literal["yes", "no", "fail"]

ANSWERS: dict[str, str] = {}


class Capabilities(TypedDict, total=False):
    web_search: Answer
    reasoning: Answer
    response_schema: Answer


class CredentialCall(TypedDict, total=False):
    api_base: str
    api_key: str
    environment_base: str
    environment_key: str


class SupportedCall(TypedDict):
    model: str


class MessageCall(TypedDict):
    model: str
    messages: list[Json]


class OptionCall(TypedDict):
    model: str
    non_default: NotRequired[Object]
    optional_params: NotRequired[Object]


class ReplyCall(TypedDict):
    reply: Object
    json_mode: NotRequired[bool]


class ChunkCall(TypedDict):
    chunk: Object


class SttCall(TypedDict, total=False):
    non_default: Object
    optional_params: Object
    drop_params: bool
    global_drop_params: bool


class CostCall(TypedDict, total=False):
    web_search_requests: int
    browser_open_requests: int
    medium_rate: str


class Input(TypedDict, total=False):
    capabilities: Capabilities
    credentials: list[CredentialCall]
    supported: list[SupportedCall]
    messages: list[MessageCall]
    options: list[OptionCall]
    replies: list[ReplyCall]
    chunks: list[ChunkCall]
    stt: list[SttCall]
    costs: list[CostCall]


class SupportedView(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...


class MapView(Protocol):
    def map_openai_params(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...


# the same five arguments as transform_response, typed through the callable attribute
class RequestView(Protocol):
    transform_request: Callable[..., Object]


# transform_response takes eleven arguments, more than a protocol stub may declare
class ReplyView(Protocol):
    transform_response: Callable[..., ModelResponse]


class HandlerView(Protocol):
    def chunk_parser(self, chunk: Object) -> ModelResponseStream: ...


class SttView(Protocol):
    def map_openai_params_stt(
        self,
        non_default_params: Object,
        optional_params: Object,
        model: str,
        *,
        drop_params: bool,
    ) -> Object: ...


CHAT: Final[Callable[..., SupportedView]] = GroqChatConfig
MAPPER: Final[Callable[..., MapView]] = GroqChatConfig
REQUEST: Final[Callable[..., RequestView]] = GroqChatConfig
REPLY: Final[Callable[..., ReplyView]] = GroqChatConfig
HANDLER: Final[Callable[..., HandlerView]] = GroqStreamHandler
STT: Final[Callable[..., SttView]] = GroqSTTConfig
COST: Final[Callable[..., float]] = cost_per_web_search_request
EMPTY: Final[Object] = {}
GROQ_PARAMS: Final[Object] = {"custom_llm_provider": "groq"}


class ServerUse(TypedDict, total=False):
    web_search_requests: int | None
    browser_open_requests: int | None


class ReplyUsage(TypedDict, total=False):
    server_tool_use: ServerUse


class ReplyRecord(TypedDict, total=False):
    service_tier: str | None
    usage: ReplyUsage


class ChunkDelta(TypedDict, total=False):
    content: str | None
    reasoning_content: str | None


class ChunkChoice(TypedDict, total=False):
    index: int
    finish_reason: str | None
    delta: ChunkDelta


class ChunkRecord(TypedDict, total=False):
    id: str | None
    object: str
    model: str | None
    choices: list[ChunkChoice]


REPLY_RECORD: Final[TypeAdapter[ReplyRecord]] = TypeAdapter(ReplyRecord)
CHUNK_RECORD: Final[TypeAdapter[ChunkRecord]] = TypeAdapter(ChunkRecord)


class LookupFailureError(Exception):
    pass


def capability_answer(name: str) -> bool:
    kind = ANSWERS.get(name)
    if kind == "yes":
        return True
    if kind == "no":
        return False
    raise LookupFailureError(f"the {name} capability lookup failed")


def web_search_answer(model: str, custom_llm_provider: str | None = None) -> bool:
    del model, custom_llm_provider
    return capability_answer("web_search")


def reasoning_answer(model: str, custom_llm_provider: str | None = None) -> bool:
    del model, custom_llm_provider
    return capability_answer("reasoning")


def response_schema_answer(model: str, custom_llm_provider: str | None = None) -> bool:
    del model, custom_llm_provider
    return capability_answer("response_schema")


def install_capabilities(capabilities: Capabilities | None) -> None:
    ANSWERS.clear()
    ANSWERS["web_search"] = "no"
    ANSWERS["reasoning"] = "no"
    ANSWERS["response_schema"] = "no"
    if capabilities is not None:
        web_search = capabilities.get("web_search")
        if web_search is not None:
            ANSWERS["web_search"] = web_search
        reasoning = capabilities.get("reasoning")
        if reasoning is not None:
            ANSWERS["reasoning"] = reasoning
        response_schema = capabilities.get("response_schema")
        if response_schema is not None:
            ANSWERS["response_schema"] = response_schema
    litellm.supports_web_search = web_search_answer
    litellm.supports_reasoning = reasoning_answer
    litellm.supports_response_schema = response_schema_answer


def credentials(calls: list[CredentialCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [credential(call) for call in calls]


def credential(call: CredentialCall) -> Json:
    os.environ.clear()
    environment_base = call.get("environment_base")
    if environment_base is not None:
        os.environ["GROQ_API_BASE"] = environment_base
    environment_key = call.get("environment_key")
    if environment_key is not None:
        os.environ["GROQ_API_KEY"] = environment_key
    resolved = litellm.get_llm_provider(
        model="groq/llama-3.3-70b-versatile",
        api_base=call.get("api_base"),
        api_key=call.get("api_key"),
    )
    return {"api_base": resolved[3], "api_key": resolved[2]}


def supported(calls: list[SupportedCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [supported_one(CHAT(), call) for call in calls]


def supported_one(view: SupportedView, call: SupportedCall) -> Json:
    try:
        params = view.get_supported_openai_params(call["model"])
    except LookupFailureError:
        return {"error": {"status": None}}
    return {"params": list[Json](params)}


def messages(calls: list[MessageCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [messages_one(REQUEST(), call) for call in calls]


def messages_one(view: RequestView, call: MessageCall) -> Json:
    try:
        body = view.transform_request(
            call["model"], call["messages"], EMPTY, GROQ_PARAMS, EMPTY
        )
    except LookupFailureError:
        return {"error": {"status": None}}
    return {"messages": body.get("messages")}


def options(calls: list[OptionCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [options_one(MAPPER(), call) for call in calls]


def options_one(view: MapView, call: OptionCall) -> Json:
    non_default = call.get("non_default")
    if non_default is None:
        non_default = EMPTY
    optional = call.get("optional_params")
    if optional is None:
        optional = EMPTY
    try:
        mapped = view.map_openai_params(
            non_default, optional, call["model"], drop_params=False
        )
    except BadRequestError as error:
        return {"error": {"status": error.status_code}}
    except LookupFailureError:
        return {"error": {"status": None}}
    return {"optional_params": mapped, "non_default": non_default}


def replies(calls: list[ReplyCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [replies_one(REPLY(), call) for call in calls]


def replies_one(view: ReplyView, call: ReplyCall) -> Json:
    response = view.transform_response(
        model="groq/llama-3.3-70b-versatile",
        raw_response=httpx.Response(200, json=call["reply"]),
        model_response=ModelResponse(),
        logging_obj=MagicMock(),
        request_data=EMPTY,
        messages=[],
        optional_params=EMPTY,
        litellm_params=EMPTY,
        encoding=None,
        json_mode=call.get("json_mode"),
    )
    record = REPLY_RECORD.validate_json(response.model_dump_json())
    usage = record.get("usage")
    server: ServerUse | None = None
    if usage is not None:
        server = usage.get("server_tool_use")
    searches: int | None = None
    opens: int | None = None
    if server is not None:
        searches = server.get("web_search_requests")
        opens = server.get("browser_open_requests")
    return {
        "service_tier": record.get("service_tier"),
        "web_search_requests": searches,
        "browser_open_requests": opens,
    }


def chunks(calls: list[ChunkCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [chunks_one(HANDLER([], sync_stream=False), call) for call in calls]


def chunks_one(view: HandlerView, call: ChunkCall) -> Json:
    try:
        chunk = view.chunk_parser(call["chunk"])
    except (OpenAIError, LookupFailureError):
        return {"error": {"status": None}}
    record = CHUNK_RECORD.validate_json(chunk.model_dump_json())
    choices: list[Json] = []
    entries = record.get("choices")
    if entries is None:
        entries = []
    for choice in entries:
        delta = choice.get("delta")
        content: str | None = None
        reasoning_content: str | None = None
        if delta is not None:
            content = delta.get("content")
            reasoning_content = delta.get("reasoning_content")
        choices.append(
            {
                "index": choice.get("index"),
                "finish_reason": choice.get("finish_reason"),
                "content": content,
                "reasoning_content": reasoning_content,
            }
        )
    return {
        "id": record.get("id"),
        "object": record.get("object"),
        "model": record.get("model"),
        "choices": choices,
    }


def stt(calls: list[SttCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [stt_one(STT(), call) for call in calls]


def stt_one(view: SttView, call: SttCall) -> Json:
    non_default = call.get("non_default")
    if non_default is None:
        non_default = EMPTY
    optional = call.get("optional_params")
    if optional is None:
        optional = EMPTY
    drop_params = call.get("drop_params")
    if drop_params is None:
        drop_params = False
    global_drop = call.get("global_drop_params")
    if global_drop is None:
        global_drop = False
    litellm.drop_params = global_drop
    try:
        mapped = view.map_openai_params_stt(
            non_default, optional, "whisper-large-v3", drop_params=drop_params
        )
    except UnsupportedParamsError as error:
        return {"error": {"status": error.status_code}}
    return {"optional_params": mapped}


def costs(calls: list[CostCall] | None) -> list[Json]:
    if calls is None:
        return []
    return [costs_one(COST, call) for call in calls]


def costs_one(calculator: Callable[..., float], call: CostCall) -> Json:
    usage = Usage(
        server_tool_use=ServerToolUse(
            web_search_requests=call.get("web_search_requests"),
            browser_open_requests=call.get("browser_open_requests"),
        )
    )
    rate = call.get("medium_rate")
    pricing: Object | None = None
    if rate is not None:
        pricing = {"search_context_size_medium": float(rate)}
    info: Object = {"search_context_cost_per_query": pricing}
    cost = calculator(usage, info)
    return {"cost": format(cost, ".12f")}


def run(input_value: Input) -> Json:
    install_capabilities(input_value.get("capabilities"))
    return {
        "credentials": credentials(input_value.get("credentials")),
        "supported": supported(input_value.get("supported")),
        "messages": messages(input_value.get("messages")),
        "options": options(input_value.get("options")),
        "replies": replies(input_value.get("replies")),
        "chunks": chunks(input_value.get("chunks")),
        "stt": stt(input_value.get("stt")),
        "costs": costs(input_value.get("costs")),
    }
