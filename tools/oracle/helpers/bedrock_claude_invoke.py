import asyncio
import time
from collections.abc import Generator
from contextlib import contextmanager
from functools import partial
from typing import Final, Literal, Protocol, TypedDict

import httpx
import litellm
import litellm.llms.bedrock.chat.invoke_transformations.anthropic_claude3_transformation as claude
from litellm.exceptions import ImageFetchError
from litellm.litellm_core_utils.prompt_templates import image_handling
from litellm.llms.bedrock.chat.invoke_transformations.anthropic_claude3_transformation import (
    AmazonAnthropicClaudeConfig,
)
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# register_model calls get_llm_provider, which prints the provider list to stdout
litellm.suppress_debug_info = True

FAILED: Final = "Error: Unable to fetch image from URL after 3 attempts. url={url}"


class TransformCall(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    headers: dict[str, str]
    fetch: dict[str, str]
    inline: bool


class Transform(TypedDict):
    function: Literal["transform"]
    model_cost: dict[str, dict[str, JsonValue]]
    calls: list[TransformCall]


class MapCall(TypedDict):
    model: str
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool


class MapParams(TypedDict):
    function: Literal["map_params"]
    model_cost: dict[str, dict[str, JsonValue]]
    calls: list[MapCall]


class ResponseCall(TypedDict):
    model: str
    raw_text: str
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    now: int


class Response(TypedDict):
    function: Literal["response"]
    calls: list[ResponseCall]


class CompletionView(TypedDict):
    reasoning_tokens: int | None
    text_tokens: int | None


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    cache_creation_input_tokens: int
    cache_read_input_tokens: int
    completion_tokens_details: CompletionView


class MessageView(TypedDict):
    content: str | None


class ChoiceView(TypedDict):
    finish_reason: str | None
    message: MessageView


class EnvelopeView(TypedDict):
    id: str
    model: str | None
    choices: list[ChoiceView]
    usage: UsageView | None


ENVELOPE: Final = TypeAdapter(EnvelopeView)


class Fetcher:
    def __init__(self, urls: dict[str, str]) -> None:
        self.urls = urls
        self.calls: list[JsonValue] = []

    def __call__(self, url: str) -> str:
        self.calls.append(url)
        value = self.urls.get(url)
        if value is None:
            raise ImageFetchError(FAILED.format(url=url))
        return value


class AsyncFetcher:
    def __init__(self, fetcher: Fetcher) -> None:
        self.fetcher = fetcher

    async def __call__(self, url: str) -> str:
        return self.fetcher(url)


class Transformer(Protocol):
    def transform_request(
        self,
        model: str,
        messages: list[dict[str, JsonValue]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    async def async_transform_request(
        self,
        model: str,
        messages: list[dict[str, JsonValue]],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_response(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/anthropic_claude3_transformation.py#L367-L394
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: "Logging",
        request_data: dict[str, JsonValue],
        messages: list[JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
        *,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


class Logging(Protocol):
    def post_call(self, **kwargs: JsonValue) -> None: ...


class Sink(Logging):
    def __init__(self) -> None:
        self.kwargs: dict[str, JsonValue] = {}

    @override
    def post_call(self, **kwargs: JsonValue) -> None:
        self.kwargs = kwargs


CONFIG: Final[Transformer] = AmazonAnthropicClaudeConfig()  # type: ignore[no-untyped-call,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/anthropic_claude3_transformation.py#L46


@contextmanager
def patched_url_fetchers(fetcher: Fetcher) -> Generator[None, None, None]:
    saved_url = claude.convert_url_to_base64  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/anthropic_claude3_transformation.py#L12
    saved_async = image_handling.async_convert_url_to_base64
    claude.convert_url_to_base64 = fetcher  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/anthropic_claude3_transformation.py#L12
    image_handling.async_convert_url_to_base64 = AsyncFetcher(fetcher)
    try:
        yield
    finally:
        claude.convert_url_to_base64 = saved_url  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/chat/invoke_transformations/anthropic_claude3_transformation.py#L12
        image_handling.async_convert_url_to_base64 = saved_async


def request_body(call: TransformCall) -> dict[str, JsonValue]:
    if call["inline"]:
        return asyncio.run(
            CONFIG.async_transform_request(
                call["model"],
                call["messages"],
                dict(call["optional_params"]),
                dict(call["litellm_params"]),
                dict(call["headers"]),
            )
        )
    return CONFIG.transform_request(
        call["model"],
        call["messages"],
        dict(call["optional_params"]),
        dict(call["litellm_params"]),
        dict(call["headers"]),
    )


def call_body(call: TransformCall) -> dict[str, JsonValue]:
    fetcher = Fetcher(call["fetch"])
    with patched_url_fetchers(fetcher):
        body = request_body(call)
    outcome: dict[str, JsonValue] = {"body": body, "fetches": fetcher.calls}
    return outcome


def transforms(argument: Transform) -> JsonValue:
    litellm.register_model(argument["model_cost"])
    calls: list[JsonValue] = [call_body(call) for call in argument["calls"]]
    return {"calls": calls}


def maps(argument: MapParams) -> JsonValue:
    litellm.register_model(argument["model_cost"])
    answers: list[JsonValue] = []
    for call in argument["calls"]:
        mapped = CONFIG.map_openai_params(
            dict(call["non_default_params"]),
            dict(call["optional_params"]),
            call["model"],
            drop_params=call["drop_params"],
        )
        answers.append({"optional_params": mapped})
    return {"calls": answers}


def reply(call: ResponseCall, sink: Sink) -> JsonValue:
    response = CONFIG.transform_response(
        call["model"],
        httpx.Response(200, content=call["raw_text"].encode(), headers={}),
        ModelResponse(id="generated"),
        sink,
        {},
        call["messages"],
        call["optional_params"],
        call["litellm_params"],
        None,
        json_mode=None,
    )
    view = ENVELOPE.validate_json(response.model_dump_json())
    choice = view["choices"][0]
    usage = view["usage"]
    outcome: dict[str, JsonValue] = {
        "id": view["id"],
        "model": view["model"],
        "finish_reason": choice["finish_reason"],
        "content": choice["message"]["content"],
        "usage": None,
    }
    if usage is not None:
        outcome["usage"] = {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
            "cache_creation_input_tokens": usage["cache_creation_input_tokens"],
            "cache_read_input_tokens": usage["cache_read_input_tokens"],
            "reasoning_tokens": usage["completion_tokens_details"]["reasoning_tokens"],
            "text_tokens": usage["completion_tokens_details"]["text_tokens"],
        }
    return outcome


def responses(argument: Response) -> JsonValue:
    sink = Sink()
    answers: list[JsonValue] = []
    for call in argument["calls"]:
        saved_time = time.time
        time.time = partial(float, call["now"])
        try:
            answers.append(reply(call, sink))
        finally:
            time.time = saved_time
    return {"calls": answers}


def run(argument: Transform | MapParams | Response) -> JsonValue:
    if argument["function"] == "transform":
        return transforms(argument)
    if argument["function"] == "map_params":
        return maps(argument)
    return responses(argument)
