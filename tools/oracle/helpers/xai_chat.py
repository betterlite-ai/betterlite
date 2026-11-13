import datetime as dt
import logging
import os
from collections.abc import Callable
from decimal import Decimal
from typing import Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.xai.chat.transformation import (
    XAIChatCompletionStreamingHandler,
    XAIChatConfig,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

SupportedParams = Callable[[str], list[str]]
MapParams = Callable[
    [dict[str, JsonValue], dict[str, JsonValue], str, bool], dict[str, JsonValue]
]
TransformRequest = Callable[
    [
        str,
        list[AllMessageValues],
        dict[str, JsonValue],
        dict[str, JsonValue],
        dict[str, JsonValue],
    ],
    dict[str, JsonValue],
]
ValidateEnvironment = Callable[
    [
        dict[str, str],
        str,
        list[AllMessageValues],
        dict[str, JsonValue],
        dict[str, JsonValue],
        str | None,
        str | None,
    ],
    dict[str, JsonValue],
]
CompleteUrl = Callable[
    [
        str | None,
        str | None,
        str,
        dict[str, JsonValue],
        dict[str, JsonValue],
        bool | None,
    ],
    str,
]
ChunkParser = Callable[[dict[str, JsonValue]], ModelResponseStream]


class ChatConfig(Protocol):
    get_supported_openai_params: SupportedParams
    map_openai_params: MapParams
    transform_request: TransformRequest
    validate_environment: ValidateEnvironment
    get_complete_url: CompleteUrl
    transform_response: Callable[..., ModelResponse]


class StreamingHandler(Protocol):
    chunk_parser: ChunkParser


CONFIG: ChatConfig = XAIChatConfig()


class Settings(TypedDict):
    api_base: str | None
    oauth_api_base: str | None
    xai_key: str | None
    environment_key: str | None


class SupportedStep(TypedDict):
    kind: Literal["supported"]
    model: str
    supports_reasoning: bool | None


class MapStep(TypedDict):
    kind: Literal["map"]
    model: str
    supports_reasoning: bool
    params: list[tuple[str, JsonValue]]
    optional_params: dict[str, JsonValue]


class RequestStep(TypedDict):
    kind: Literal["request"]
    model: str
    messages: list[AllMessageValues]
    optional_params: dict[str, JsonValue]


class EnvironmentStep(TypedDict):
    kind: Literal["environment"]
    model: str
    headers: dict[str, str]
    api_key: str | None
    use_xai_oauth: bool


class UrlStep(TypedDict):
    kind: Literal["url"]
    api_base: str | None
    api_key: str | None
    use_xai_oauth: bool


class ReplyStep(TypedDict):
    kind: Literal["reply"]
    model: str
    body: str


class ChunkStep(TypedDict):
    kind: Literal["chunk"]
    chunk: dict[str, JsonValue]


Step = (
    SupportedStep
    | MapStep
    | RequestStep
    | EnvironmentStep
    | UrlStep
    | ReplyStep
    | ChunkStep
)


class Input(TypedDict):
    settings: Settings
    steps: list[Step]


class MessageView(TypedDict):
    tool_calls: NotRequired[list[JsonValue] | None]


class ChoiceView(TypedDict):
    finish_reason: str | None
    message: MessageView


class PromptDetailsView(TypedDict):
    web_search_requests: NotRequired[int | None]


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    cost: NotRequired[float | None]
    prompt_tokens_details: NotRequired[PromptDetailsView | None]


class ReplyView(TypedDict):
    choices: list[ChoiceView]
    usage: UsageView


class DeltaView(TypedDict):
    role: NotRequired[str | None]


class StreamChoiceView(TypedDict):
    index: int
    finish_reason: NotRequired[str | None]
    delta: DeltaView


class StreamView(TypedDict):
    choices: list[StreamChoiceView]
    usage: NotRequired[UsageView | None]


class Scripted:
    def __init__(self, *, outcome: bool | None) -> None:
        self.outcome = outcome

    def __call__(self, model: str, custom_llm_provider: str | None = None) -> bool:
        if custom_llm_provider is None:
            custom_llm_provider = "xai"
        if model == "" or self.outcome is None:
            error = LookupError("the capability lookup failed")
            raise error from None
        return self.outcome


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def install(settings: Settings) -> None:
    os.environ.pop("XAI_API_KEY", None)
    os.environ.pop("XAI_API_BASE", None)
    os.environ.pop("XAI_OAUTH_API_BASE", None)
    if settings["environment_key"] is not None:
        os.environ["XAI_API_KEY"] = settings["environment_key"]
    if settings["api_base"] is not None:
        os.environ["XAI_API_BASE"] = settings["api_base"]
    if settings["oauth_api_base"] is not None:
        os.environ["XAI_OAUTH_API_BASE"] = settings["oauth_api_base"]
    litellm.xai_key = settings["xai_key"]


def supported_step(step: SupportedStep) -> JsonValue:
    litellm.supports_reasoning = Scripted(outcome=step["supports_reasoning"])
    supported: list[JsonValue] = [*CONFIG.get_supported_openai_params(step["model"])]
    return {"supported": supported}


def map_step(step: MapStep) -> JsonValue:
    drop_params = False
    mapped = CONFIG.map_openai_params(
        dict(step["params"]),
        dict(step["optional_params"]),
        step["model"],
        drop_params,
    )
    return {"mapped": mapped}


def request_step(step: RequestStep) -> JsonValue:
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    try:
        body = CONFIG.transform_request(
            step["model"],
            step["messages"],
            dict(step["optional_params"]),
            {},
            {},
        )
    finally:
        logger.removeHandler(warnings)
    return {"body": body, "warnings": len(warnings.messages)}


def environment_step(step: EnvironmentStep) -> JsonValue:
    headers = CONFIG.validate_environment(
        dict(step["headers"]),
        step["model"],
        [],
        {},
        {"use_xai_oauth": step["use_xai_oauth"]},
        step["api_key"],
        None,
    )
    rendered: dict[str, JsonValue] = dict(headers.items())
    return {"headers": rendered}


def url_step(step: UrlStep) -> JsonValue:
    _, _provider, api_key, api_base = litellm.get_llm_provider(
        model="xai/grok-4",
        api_base=step["api_base"],
        api_key=step["api_key"],
    )
    url = CONFIG.get_complete_url(
        api_base,
        api_key,
        "grok-4",
        {},
        {"use_xai_oauth": step["use_xai_oauth"]},
        None,
    )
    return {"url": url, "api_base": api_base, "api_key": api_key}


def tool_count(choice: ChoiceView) -> int:
    calls = choice["message"].get("tool_calls")
    if calls is None:
        return 0
    return len(calls)


def usage_view(usage: UsageView) -> dict[str, JsonValue]:
    cost = usage.get("cost")
    details = usage.get("prompt_tokens_details")
    web_search: int | None = None
    if details is not None:
        web_search = details.get("web_search_requests")
    return {
        "prompt_tokens": usage["prompt_tokens"],
        "completion_tokens": usage["completion_tokens"],
        "total_tokens": usage["total_tokens"],
        "cost": None if cost is None else format(Decimal(repr(cost)).normalize(), "f"),
        "web_search_requests": web_search,
    }


def reply_step(step: ReplyStep) -> JsonValue:
    logging_obj = Logging(
        model="xai/grok-4",
        messages=[],
        stream=False,
        call_type="completion",
        start_time=dt.datetime.now(dt.UTC),
        litellm_call_id="oracle-call",
        function_id="oracle-function",
    )
    transformed = CONFIG.transform_response(
        model=step["model"],
        raw_response=httpx.Response(200, content=step["body"]),
        model_response=ModelResponse(),
        logging_obj=logging_obj,
        request_data={},
        messages=[],
        optional_params={},
        litellm_params={},
        encoding=None,
    )
    view = TypeAdapter(ReplyView).validate_json(transformed.model_dump_json())
    choices: list[JsonValue] = [
        {
            "finish_reason": choice["finish_reason"],
            "tool_calls": tool_count(choice),
        }
        for choice in view["choices"]
    ]
    return {"choices": choices, "usage": usage_view(view["usage"])}


def streaming_handler() -> StreamingHandler:
    return XAIChatCompletionStreamingHandler(
        streaming_response=iter(()), sync_stream=False
    )


def chunk_step(step: ChunkStep) -> JsonValue:
    parsed = streaming_handler().chunk_parser(dict(step["chunk"]))
    view = TypeAdapter(StreamView).validate_json(parsed.model_dump_json())
    choices: list[JsonValue] = [
        {"index": choice["index"], "finish_reason": choice.get("finish_reason")}
        for choice in view["choices"]
    ]
    usage: JsonValue = None
    if "usage" in view:
        parsed_usage = view["usage"]
        usage = None if parsed_usage is None else usage_view(parsed_usage)
    return {"choices": choices, "usage": usage}


def run(input_value: Input) -> JsonValue:
    install(input_value["settings"])
    results: list[JsonValue] = []
    for step in input_value["steps"]:
        if step["kind"] == "supported":
            results.append(supported_step(step))
        elif step["kind"] == "map":
            results.append(map_step(step))
        elif step["kind"] == "request":
            results.append(request_step(step))
        elif step["kind"] == "environment":
            results.append(environment_step(step))
        elif step["kind"] == "url":
            results.append(url_step(step))
        elif step["kind"] == "reply":
            results.append(reply_step(step))
        elif step["kind"] == "chunk":
            results.append(chunk_step(step))
        else:
            raise ValueError("unknown xai_chat step")
    return results
