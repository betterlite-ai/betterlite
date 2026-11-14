import datetime as dt
import logging
import os
from collections.abc import Callable
from decimal import Decimal
from typing import Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.xai.chat.transformation import (
    XAIChatCompletionStreamingHandler,
    XAIChatConfig,
)
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

Params = dict[str, JsonValue]
Messages = list[JsonValue]


class ChatConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    map_openai_params: Callable[[Params, Params, str, bool], Params]
    transform_request: Callable[[str, Messages, Params, Params, Params], Params]
    validate_environment: Callable[
        [dict[str, str], str, Messages, Params, Params, str | None, str | None],
        Params,
    ]
    get_complete_url: Callable[[str | None, str | None, str, Params, Params, bool | None], str]
    transform_response: Callable[
        [str, httpx.Response, ModelResponse, Logging, Params, Messages, Params, Params, None],
        ModelResponse,
    ]


class StreamingHandler(Protocol):
    def chunk_parser(self, chunk: Params) -> ModelResponseStream: ...


CONFIG: ChatConfig = XAIChatConfig()  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005


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
    messages: list[JsonValue]
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


Step = SupportedStep | MapStep | RequestStep | EnvironmentStep | UrlStep | ReplyStep | ChunkStep


class Input(TypedDict):
    settings: Settings
    steps: list[Step]


class MessageView(TypedDict):
    tool_calls: NotRequired[list[JsonValue] | None]


class ChoiceView(TypedDict):
    finish_reason: str | None
    provider_specific_fields: dict[str, JsonValue] | None
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


class StreamChoiceView(TypedDict):
    index: int
    finish_reason: NotRequired[str | None]


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

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def install(settings: Settings) -> None:
    wanted = {
        "XAI_API_KEY": settings["environment_key"],
        "XAI_API_BASE": settings["api_base"],
        "XAI_OAUTH_API_BASE": settings["oauth_api_base"],
    }
    for name, value in wanted.items():
        if value is None:
            os.environ.pop(name, None)
        else:
            os.environ[name] = value
    litellm.xai_key = settings["xai_key"]


def supported_step(step: SupportedStep) -> JsonValue:
    litellm.supports_reasoning = Scripted(outcome=step["supports_reasoning"])
    supported: list[JsonValue] = [*CONFIG.get_supported_openai_params(step["model"])]
    return {"supported": supported}


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


def reply_view(step: ReplyStep) -> ReplyView:
    logging_obj = Logging(
        model="xai/grok-4",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=dt.datetime.now(dt.UTC),
        litellm_call_id="oracle-call",
        function_id="oracle-function",
    )
    response = httpx.Response(200, content=step["body"])
    transformed = CONFIG.transform_response(
        step["model"], response, ModelResponse(), logging_obj, {}, [], {}, {}, None
    )
    return TypeAdapter(ReplyView).validate_json(transformed.model_dump_json())


def reply_step(step: ReplyStep) -> JsonValue:
    view = reply_view(step)
    choices: list[JsonValue] = []
    for choice in view["choices"]:
        fields = choice["provider_specific_fields"]
        calls = choice["message"].get("tool_calls")
        reading: dict[str, JsonValue] = {"finish_reason": choice["finish_reason"]}
        reading["provider_specific_fields"] = {} if fields is None else fields
        reading["tool_calls"] = 0 if calls is None else len(calls)
        choices.append(reading)
    return {"choices": choices, "usage": usage_view(view["usage"])}


def chunk_step(step: ChunkStep) -> JsonValue:
    handler: StreamingHandler = XAIChatCompletionStreamingHandler(
        streaming_response=iter(()), sync_stream=False
    )
    parsed = handler.chunk_parser(dict(step["chunk"]))  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/xai/chat/transformation.py#L408
    view = TypeAdapter(StreamView).validate_json(parsed.model_dump_json())
    choices: list[JsonValue] = [
        {"index": c["index"], "finish_reason": c.get("finish_reason")} for c in view["choices"]
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
            drop_params = False
            mapped = CONFIG.map_openai_params(
                dict(step["params"]), dict(step["optional_params"]), step["model"], drop_params
            )
            results.append({"mapped": mapped})
        elif step["kind"] == "request":
            results.append(request_step(step))
        elif step["kind"] == "environment":
            oauth: Params = {"use_xai_oauth": step["use_xai_oauth"]}
            headers = CONFIG.validate_environment(
                dict(step["headers"]), step["model"], [], {}, oauth, step["api_key"], None
            )
            rendered: dict[str, JsonValue] = dict(headers.items())
            results.append({"headers": rendered})
        elif step["kind"] == "url":
            _, _provider, api_key, api_base = get_llm_provider(
                model="xai/grok-4", api_base=step["api_base"], api_key=step["api_key"]
            )
            oauth = {"use_xai_oauth": step["use_xai_oauth"]}
            url = CONFIG.get_complete_url(api_base, api_key, "grok-4", {}, oauth, None)
            results.append({"url": url, "api_base": api_base, "api_key": api_key})
        elif step["kind"] == "reply":
            results.append(reply_step(step))
        elif step["kind"] == "chunk":
            results.append(chunk_step(step))
        else:
            raise ValueError("unknown xai_chat step")
    return results
