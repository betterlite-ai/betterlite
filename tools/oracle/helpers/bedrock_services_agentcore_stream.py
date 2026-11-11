import os

os.environ["LITELLM_RUST"] = "false"

import asyncio
from collections.abc import AsyncIterator, Awaitable, Callable, Iterator
from types import SimpleNamespace
from typing import TYPE_CHECKING, Literal, TypeAlias, cast
from unittest import mock

import httpx
from litellm.llms.bedrock.chat.agentcore import transformation
from litellm.llms.bedrock.chat.agentcore.transformation import AmazonAgentCoreConfig
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import LlmProviders, ModelResponseStream, Usage
from pydantic import BaseModel, ConfigDict

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


class Logging:
    def __init__(self) -> None:
        self.model_call_details: dict[str, object] = {"litellm_params": {}}
        self.messages: object = None
        self.stream_options: object = None

    def post_call(self, **_kwargs: object) -> None:
        pass


def _ids(values: list[str]) -> SimpleNamespace:
    remaining = iter(values)

    def uuid4() -> str:
        return next(remaining)

    return SimpleNamespace(uuid4=uuid4)


class SyncFixture(HTTPHandler):
    def __init__(self, response: httpx.Response) -> None:
        self.reply = response

    def post(  # pyright: ignore[reportIncompatibleMethodOverride]
        self, _url: str, **_kwargs: object
    ) -> httpx.Response:
        return self.reply


class AsyncFixture(AsyncHTTPHandler):
    def __init__(self, response: httpx.Response) -> None:
        self.reply = response

    async def post(  # pyright: ignore[reportIncompatibleMethodOverride]
        self, _url: str, **_kwargs: object
    ) -> httpx.Response:
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


class Attempt:
    def __init__(self, model: str, ids: list[str], scenario: Scenario, path: Path) -> None:
        self.model = model
        self.ids = ids
        self.scenario: Scenario = scenario
        self.path: Path = path
        self.factories: list[Json] = []


def _parameters(attempt: Attempt) -> dict[str, object]:
    scenario = attempt.scenario
    signed = None if scenario.signed_json_body is None else bytes(scenario.signed_json_body)
    return {
        "model": attempt.model,
        "custom_llm_provider": "openai",
        "logging_obj": Logging(),
        "api_base": scenario.api_base,
        "headers": dict(scenario.request_headers),
        "data": scenario.data,
        "messages": scenario.messages,
        "json_mode": None,
        "signed_json_body": signed,
        "litellm_params": {},
    }


async def _call(
    attempt: Attempt, client: SyncFixture | AsyncFixture | None
) -> tuple[Json, list[ModelResponseStream]]:
    scenario: Scenario = attempt.scenario
    path: Path = attempt.path

    def sync_factory(params: dict[str, object] | None = None) -> HTTPHandler:
        if params != {}:
            raise ValueError("the default sync client was built with parameters")
        attempt.factories.append({"params": cast(Json, params)})
        return SyncFixture(_response(scenario, path))

    def async_factory(
        llm_provider: object, params: dict[str, object] | None = None, shared_session: object = None
    ) -> AsyncHTTPHandler:
        del shared_session
        if params != {}:
            raise ValueError("the default async client was built with parameters")
        if llm_provider != LlmProviders.BEDROCK:
            raise ValueError("the default async client was built for another provider")
        attempt.factories.append({"params": cast(Json, params), "provider": str(llm_provider)})
        return AsyncFixture(_response(scenario, path))

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
            chunks = list(cast("Iterator[ModelResponseStream]", wrapper.completion_stream))
        else:
            method = cast(  # pyright: ignore[reportUnknownMemberType]
                "Callable[..., Awaitable[object]]", config.get_async_custom_stream_wrapper
            )
            wrapper = cast(
                "CustomStreamWrapper",
                await method(client=cast("AsyncHTTPHandler", client), **parameters),
            )
            stream = cast("AsyncIterator[ModelResponseStream]", wrapper.completion_stream)
            chunks = [chunk async for chunk in stream]
    return cast(Json, wrapper.custom_llm_provider), chunks


def _failure(handler: str, error: Json) -> Json:
    return {
        "handler": handler,
        "provider": None,
        "failed": True,
        "chunks": [],
        "error": error,
    }


async def _outcome(model: str, ids: list[str], scenario: Scenario, path: Path) -> Json:
    attempt = Attempt(model, ids, scenario, path)
    supplied = scenario.supplied == path
    handler = "supplied" if supplied else "default"
    if scenario.supplied == "sync":
        client: SyncFixture | AsyncFixture | None = SyncFixture(_response(scenario, path))
    elif scenario.supplied == "async":
        client = AsyncFixture(_response(scenario, path))
    else:
        client = None
    try:
        provider, chunks = await _call(attempt, client)
    except BedrockError as error:
        return _failure(handler, _provider_error(error))
    except Exception:  # noqa: BLE001 - a case compares the failure category, not the Python class
        return _failure(handler, {"kind": "invalid"})
    if supplied and attempt.factories:
        raise ValueError("a supplied client still built the default client")
    if not supplied and not attempt.factories:
        raise ValueError("the default client was never built")
    return {
        "handler": handler,
        "provider": provider,
        "failed": False,
        "chunks": [_chunk(chunk) for chunk in chunks],
        "error": None,
    }


async def execute(call: Input) -> Json:
    sync: list[Json] = []
    asynchronous: list[Json] = []
    for scenario in call.scenarios:
        sync.append(await _outcome(call.model, call.ids, scenario, "sync"))
        asynchronous.append(await _outcome(call.model, call.ids, scenario, "async"))
    return {"sync": sync, "async": asynchronous}


def run(argument: Json) -> Json:
    return asyncio.run(execute(Input.model_validate(argument)))
