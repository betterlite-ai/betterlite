import os

os.environ["LITELLM_RUST"] = "false"

import asyncio
import json
from collections.abc import AsyncIterator, Awaitable, Callable, Iterator
from types import SimpleNamespace
from typing import TYPE_CHECKING, Literal, TypeAlias, cast
from unittest import mock

import httpx
import litellm
from litellm.llms.bedrock.chat.agentcore import transformation
from litellm.llms.bedrock.chat.agentcore.transformation import AmazonAgentCoreConfig
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import LlmProviders, ModelResponseStream, Usage
from pydantic import BaseModel, ConfigDict

litellm.suppress_debug_info = True

if TYPE_CHECKING:
    from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Path: TypeAlias = Literal["sync", "async"]
API_BASE = "https://bedrock-agentcore.example/runtime"
HANDLERS = "litellm.llms.custom_httpx.http_handler"


class Scenario(BaseModel):
    model_config = ConfigDict(extra="forbid")
    reads: list[str] = []
    status: int = 200
    headers: list[tuple[str, str]] = []
    api_base: str = API_BASE
    request_headers: list[tuple[str, str]] = []
    data: dict[str, object] = {}
    signed_json_body: list[int] | None = None
    messages: list[dict[str, object]] = []
    supplied: Path | None = None


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str
    ids: list[str]
    scenarios: list[Scenario]


class Attempt:
    def __init__(self, model: str, ids: list[str], scenario: Scenario, path: Path) -> None:
        self.model = model
        self.ids = ids
        self.scenario: Scenario = scenario
        self.path: Path = path
        self.factories: list[Json] = []
        self.posts: list[Json] = []
        self.post_calls: list[Json] = []
        self.chunks: list[ModelResponseStream] = []


class Logging:
    def __init__(self, attempt: Attempt) -> None:
        self.attempt = attempt
        self.model_call_details: dict[str, object] = {"litellm_params": {}}
        self.messages: object = None
        self.stream_options: object = None

    def post_call(
        self,
        input: Json,  # noqa: A002 - LiteLLM passes this keyword
        api_key: str,
        original_response: str,
        additional_args: dict[str, Json],
    ) -> None:
        self.attempt.post_calls.append(
            {
                "input": input,
                "api_key": api_key,
                "original_response": original_response,
                "complete_input_dict": additional_args["complete_input_dict"],
            }
        )


def _ids(values: list[str]) -> SimpleNamespace:
    remaining = iter(values)

    def uuid4() -> str:
        return next(remaining)

    return SimpleNamespace(uuid4=uuid4)


def _post_record(
    label: str, url: str, headers: dict[str, str], data: str | bytes, *, stream: bool
) -> Json:
    raw = data.encode() if isinstance(data, str) else data
    try:
        body = cast(Json, json.loads(raw))
    except ValueError:
        body = cast(Json, list(raw))
    pairs = sorted([[name.lower(), value] for name, value in headers.items()])
    return {
        "handler": label,
        "url": url,
        "headers": cast(Json, pairs),
        "body": body,
        "stream": stream,
    }


class SyncFixture(HTTPHandler):
    def __init__(self, response: httpx.Response, label: str, attempt: Attempt) -> None:
        self.reply = response
        self.label = label
        self.attempt = attempt

    def post(  # pyright: ignore[reportIncompatibleMethodOverride]
        self,
        url: str,
        headers: dict[str, str],
        data: str | bytes,
        stream: bool,  # noqa: FBT001
        logging_obj: object,
    ) -> httpx.Response:
        del logging_obj
        self.attempt.posts.append(_post_record(self.label, url, headers, data, stream=stream))
        return self.reply


class AsyncFixture(AsyncHTTPHandler):
    def __init__(self, response: httpx.Response, label: str, attempt: Attempt) -> None:
        self.reply = response
        self.label = label
        self.attempt = attempt

    async def post(  # pyright: ignore[reportIncompatibleMethodOverride]
        self,
        url: str,
        headers: dict[str, str],
        data: str | bytes,
        stream: bool,  # noqa: FBT001
        logging_obj: object,
    ) -> httpx.Response:
        del logging_obj
        self.attempt.posts.append(_post_record(self.label, url, headers, data, stream=stream))
        return self.reply


class SyncReads(httpx.SyncByteStream):
    def __init__(self, reads: list[str]) -> None:
        self.reads = reads

    def __iter__(self) -> Iterator[bytes]:
        for read in self.reads:
            yield read.encode()


class AsyncReads(httpx.AsyncByteStream):
    def __init__(self, reads: list[str]) -> None:
        self.reads = reads

    async def __aiter__(self) -> AsyncIterator[bytes]:
        for read in self.reads:
            yield read.encode()


def _response(scenario: Scenario, path: Path) -> httpx.Response:
    stream = SyncReads(scenario.reads) if path == "sync" else AsyncReads(scenario.reads)
    return httpx.Response(scenario.status, headers=scenario.headers, stream=stream)


def _chunk(chunk: ModelResponseStream) -> Json:
    choice = chunk.choices[0]
    usage = cast("Usage | None", getattr(chunk, "usage", None))
    return {
        "id": chunk.id,
        "object": chunk.object,
        "created": chunk.created,
        "model": chunk.model,
        "choices": [
            {
                "index": choice.index,
                "finish_reason": choice.finish_reason,
                "delta": {
                    "content": choice.delta.content,
                    "role": cast(Json, getattr(choice.delta, "role", None)),
                },
            }
        ],
        "usage": None
        if usage is None
        else {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    }


def _provider_error(error: BedrockError) -> Json:
    headers = cast("dict[str, object] | httpx.Headers | None", error.headers)
    if isinstance(headers, httpx.Headers):
        pairs = sorted([list(pair) for pair in headers.multi_items()])
    elif isinstance(headers, dict):
        pairs = sorted([[name, value] for name, value in headers.items()])
    else:
        pairs = None
    return {
        "kind": "provider",
        "status_code": error.status_code,
        "headers": cast(Json, pairs),
        "response_status": error.response.status_code,
    }


def _parameters(attempt: Attempt) -> dict[str, object]:
    scenario = attempt.scenario
    signed = None if scenario.signed_json_body is None else bytes(scenario.signed_json_body)
    return {
        "model": attempt.model,
        "custom_llm_provider": "openai",
        "logging_obj": Logging(attempt),
        "api_base": scenario.api_base,
        "headers": dict(scenario.request_headers),
        "data": scenario.data,
        "messages": scenario.messages,
        "json_mode": None,
        "signed_json_body": signed,
        "litellm_params": {},
    }


async def _call(attempt: Attempt, client: SyncFixture | AsyncFixture | None) -> Json:
    scenario: Scenario = attempt.scenario
    path: Path = attempt.path

    def sync_factory(params: dict[str, object] | None = None) -> HTTPHandler:
        if params != {}:
            raise ValueError("the default sync client was built with parameters")
        attempt.factories.append({"params": cast(Json, params)})
        return SyncFixture(_response(scenario, path), "default", attempt)

    def async_factory(
        llm_provider: object, params: dict[str, object] | None = None, shared_session: object = None
    ) -> AsyncHTTPHandler:
        del shared_session
        if params != {}:
            raise ValueError("the default async client was built with parameters")
        if llm_provider != LlmProviders.BEDROCK:
            raise ValueError("the default async client was built for another provider")
        attempt.factories.append({"params": cast(Json, params), "provider": str(llm_provider)})
        return AsyncFixture(_response(scenario, path), "default", attempt)

    parameters = _parameters(attempt)
    config = AmazonAgentCoreConfig()
    with (
        mock.patch.object(transformation, "uuid", _ids(attempt.ids)),
        mock.patch(f"{HANDLERS}._get_httpx_client", sync_factory),
        mock.patch(f"{HANDLERS}.get_async_httpx_client", async_factory),
    ):
        wrapper: CustomStreamWrapper
        if path == "sync":
            method = cast("Callable[..., object]", config.get_sync_custom_stream_wrapper)  # pyright: ignore[reportUnknownMemberType]
            wrapper = cast("CustomStreamWrapper", method(client=client, **parameters))
            for chunk in cast("Iterator[ModelResponseStream]", wrapper.completion_stream):
                attempt.chunks.append(chunk)
        else:
            method = cast(  # pyright: ignore[reportUnknownMemberType]
                "Callable[..., Awaitable[object]]", config.get_async_custom_stream_wrapper
            )
            wrapper = cast(
                "CustomStreamWrapper",
                await method(client=cast("AsyncHTTPHandler", client), **parameters),
            )
            stream = cast("AsyncIterator[ModelResponseStream]", wrapper.completion_stream)
            async for chunk in stream:
                attempt.chunks.append(chunk)
    return cast(Json, wrapper.custom_llm_provider)


def _result(attempt: Attempt, provider: Json, error: Json) -> Json:
    return {
        "provider": provider if error is None else None,
        "failed": error is not None,
        "chunks": [_chunk(chunk) for chunk in attempt.chunks],
        "error": error,
        "posts": attempt.posts,
        "post_calls": attempt.post_calls,
    }


async def _outcome(model: str, ids: list[str], scenario: Scenario, path: Path) -> Json:
    attempt = Attempt(model, ids, scenario, path)
    supplied = scenario.supplied == path
    if scenario.supplied == "sync":
        client: SyncFixture | AsyncFixture | None = SyncFixture(
            _response(scenario, path), "supplied", attempt
        )
    elif scenario.supplied == "async":
        client = AsyncFixture(_response(scenario, path), "supplied", attempt)
    else:
        client = None
    try:
        provider = await _call(attempt, client)
    except BedrockError as error:
        return _result(attempt, None, _provider_error(error))
    except Exception:  # noqa: BLE001 - a case compares the failure category, not the Python class
        return _result(attempt, None, {"kind": "invalid"})
    if supplied and attempt.factories:
        raise ValueError("a supplied client still built the default client")
    if not supplied and not attempt.factories:
        raise ValueError("the default client was never built")
    return _result(attempt, provider, None)


async def execute(call: Input) -> Json:
    sync: list[Json] = []
    asynchronous: list[Json] = []
    for scenario in call.scenarios:
        sync.append(await _outcome(call.model, call.ids, scenario, "sync"))
        asynchronous.append(await _outcome(call.model, call.ids, scenario, "async"))
    return {"sync": sync, "async": asynchronous}


def run(argument: Json) -> Json:
    return asyncio.run(execute(Input.model_validate(argument)))
