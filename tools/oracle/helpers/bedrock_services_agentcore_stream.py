import os

os.environ["LITELLM_RUST"] = "false"

import asyncio
import json
from collections.abc import AsyncIterator, Iterator
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
    from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
    from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Path: TypeAlias = Literal["sync", "async"]
DEFAULT_API_BASE = "https://bedrock-agentcore.example/runtime"
CUSTOM_PROVIDER = "openai"
TRANSFORMATION_UUID = "litellm.llms.bedrock.chat.agentcore.transformation.uuid"
HTTP_HANDLER = "litellm.llms.custom_httpx.http_handler"


class Scenario(BaseModel):
    model_config = ConfigDict(extra="forbid")
    reads: list[str] = []
    status: int = 200
    headers: list[tuple[str, str]] = []
    api_base: str = DEFAULT_API_BASE
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


class Recorded(BaseModel):
    model_config = ConfigDict(extra="forbid")
    url: str
    headers: dict[str, str]
    data_text: str | None
    data_bytes: list[int] | None
    stream: bool
    has_logging: bool
    has_timeout: bool


class Logging:
    def __init__(self) -> None:
        self.model_call_details: dict[str, object] = {
            "litellm_params": {},
            "custom_llm_provider": "bedrock",
        }
        self.messages: object = None
        self.stream_options: object = None
        self.calls: list[Object] = []

    def post_call(self, **kwargs: object) -> None:
        recorded: Object = {name: _json_value(value) for name, value in kwargs.items()}
        self.calls.append(recorded)

    def projection(self) -> Json:
        if not self.calls:
            return None
        recorded = self.calls[0]
        additional = recorded.get("additional_args")
        complete: Json = None
        if isinstance(additional, dict):
            complete = cast("dict[str, Json]", additional).get("complete_input_dict")
        return {
            "input": recorded.get("input"),
            "api_key": recorded.get("api_key"),
            "original_response": recorded.get("original_response"),
            "complete_input_dict": complete,
        }


def _json_value(value: object) -> Json:
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    return cast("Json", json.loads(json.dumps(value)))


class Ids:
    def __init__(self, values: list[str]) -> None:
        self.values = list(values)
        self.index = 0

    def uuid4(self) -> str:
        if self.index >= len(self.values):
            raise IndexError("the case ran out of ids")
        value = self.values[self.index]
        self.index += 1
        return value


def _record(url: str, kwargs: dict[str, object], record: list[Recorded]) -> None:
    headers = kwargs.get("headers")
    data = kwargs.get("data")
    record.append(
        Recorded(
            url=url,
            headers={} if headers is None else cast("dict[str, str]", headers),
            data_text=data if isinstance(data, str) else None,
            data_bytes=None if not isinstance(data, bytes) else list(data),
            stream=kwargs.get("stream") is True,
            has_logging=kwargs.get("logging_obj") is not None,
            has_timeout=kwargs.get("timeout") is not None,
        )
    )


class SyncFixture(HTTPHandler):
    def __init__(self, response: httpx.Response, record: list[Recorded]) -> None:
        self.reply = response
        self.record = record

    def post(  # pyright: ignore[reportIncompatibleMethodOverride]
        self, url: str, **kwargs: object
    ) -> httpx.Response:
        _record(url, kwargs, self.record)
        return self.reply


class AsyncFixture(AsyncHTTPHandler):
    def __init__(self, response: httpx.Response, record: list[Recorded]) -> None:
        self.reply = response
        self.record = record

    async def post(  # pyright: ignore[reportIncompatibleMethodOverride]
        self, url: str, **kwargs: object
    ) -> httpx.Response:
        _record(url, kwargs, self.record)
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
    if path == "sync":
        stream: httpx.SyncByteStream | httpx.AsyncByteStream = SyncReads(scenario.reads)
    else:
        stream = AsyncReads(scenario.reads)
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


def _headers(headers: dict[str, object] | httpx.Headers | None) -> Json:
    if headers is None:
        return None
    if isinstance(headers, httpx.Headers):
        return cast("Json", sorted([list(pair) for pair in headers.multi_items()]))
    return cast("Json", sorted([[name, value] for name, value in headers.items()]))


def _provider_error(error: BedrockError) -> Json:
    headers = cast("dict[str, object] | httpx.Headers | None", error.headers)
    return {
        "kind": "provider",
        "status_code": error.status_code,
        "headers": _headers(headers),
        "response_status": error.response.status_code,
    }


class Attempt:
    def __init__(self) -> None:
        self.record: list[Recorded] = []
        self.factories: list[Object] = []
        self.logging = Logging()


def _request(record: list[Recorded]) -> Json:
    if not record:
        return None
    entry = record[0]
    if entry.has_timeout:
        raise ValueError("the AgentCore stream call passed an explicit timeout")
    if not entry.has_logging:
        raise ValueError("the AgentCore stream post call did not receive logging_obj")
    body: Json
    if entry.data_bytes is not None:
        body = cast("Json", {"signed": entry.data_bytes})
    else:
        body = {"json": cast("Json", json.loads(entry.data_text or "null"))}
    return {
        "url": entry.url,
        "headers": cast("Json", sorted([[name, value] for name, value in entry.headers.items()])),
        "body": body,
        "stream": entry.stream,
    }


def _supplied_client(
    scenario: Scenario, path: Path, attempt: Attempt
) -> SyncFixture | AsyncFixture | None:
    if scenario.supplied == "sync":
        return SyncFixture(_response(scenario, path), attempt.record)
    if scenario.supplied == "async":
        return AsyncFixture(_response(scenario, path), attempt.record)
    return None


async def _call(
    call: Input,
    scenario: Scenario,
    path: Path,
    attempt: Attempt,
    client: SyncFixture | AsyncFixture | None,
) -> tuple[Json, list[ModelResponseStream]]:
    def sync_factory(params: dict[str, object] | None = None) -> HTTPHandler:
        if params != {}:
            raise ValueError("the default sync client was built with parameters")
        attempt.factories.append({"params": cast(Json, params)})
        return SyncFixture(_response(scenario, path), attempt.record)

    def async_factory(
        llm_provider: object,
        params: dict[str, object] | None = None,
        shared_session: object = None,
    ) -> AsyncHTTPHandler:
        del shared_session
        if params != {}:
            raise ValueError("the default async client was built with parameters")
        if llm_provider != LlmProviders.BEDROCK:
            raise ValueError("the default async client was built for another provider")
        attempt.factories.append({"params": cast(Json, params), "provider": str(llm_provider)})
        return AsyncFixture(_response(scenario, path), attempt.record)

    signed = None if scenario.signed_json_body is None else bytes(scenario.signed_json_body)
    logging = cast("LiteLLMLoggingObj", attempt.logging)
    config = AmazonAgentCoreConfig()
    wrapper: CustomStreamWrapper
    with (
        mock.patch.object(transformation, "uuid", Ids(call.ids)),
        mock.patch(f"{HTTP_HANDLER}._get_httpx_client", sync_factory),
        mock.patch(f"{HTTP_HANDLER}.get_async_httpx_client", async_factory),
    ):
        if path == "sync":
            wrapper = config.get_sync_custom_stream_wrapper(  # pyright: ignore[reportUnknownMemberType]

                model=call.model,
                custom_llm_provider=CUSTOM_PROVIDER,
                logging_obj=logging,
                api_base=scenario.api_base,
                headers=dict(scenario.request_headers),
                data=scenario.data,
                messages=cast("list[object]", scenario.messages),
                client=client,
                json_mode=None,
                signed_json_body=signed,
                litellm_params={},
            )
            chunks = list(cast("Iterator[ModelResponseStream]", wrapper.completion_stream))
        else:
            wrapper = await config.get_async_custom_stream_wrapper(  # pyright: ignore[reportUnknownMemberType]
                model=call.model,
                custom_llm_provider=CUSTOM_PROVIDER,
                logging_obj=logging,
                api_base=scenario.api_base,
                headers=dict(scenario.request_headers),
                data=scenario.data,
                messages=cast("list[object]", scenario.messages),
                client=cast("AsyncHTTPHandler", client),
                json_mode=None,
                signed_json_body=signed,
                litellm_params={},
            )
            chunks = [
                chunk
                async for chunk in cast(
                    "AsyncIterator[ModelResponseStream]", wrapper.completion_stream
                )
            ]
    return cast(Json, wrapper.custom_llm_provider), chunks


def _failure(handler: str, attempt: Attempt, error: Json) -> Json:
    return {
        "handler": handler,
        "provider": None,
        "failed": True,
        "chunks": [],
        "post": _request(attempt.record),
        "post_call": attempt.logging.projection(),
        "error": error,
    }


def _check_selection(*, supplied: bool, attempt: Attempt) -> None:
    if supplied and attempt.factories:
        raise ValueError("a supplied client still built the default client")
    if not supplied and not attempt.factories:
        raise ValueError("the default client was never built")


async def _outcome(call: Input, scenario: Scenario, path: Path) -> Json:
    attempt = Attempt()
    supplied = scenario.supplied == path
    handler = "supplied" if supplied else "default"
    client = _supplied_client(scenario, path, attempt)
    try:
        provider, chunks = await _call(call, scenario, path, attempt, client)
    except BedrockError as error:
        return _failure(handler, attempt, _provider_error(error))
    except Exception:  # noqa: BLE001 - a case compares the failure category, not the Python class
        return _failure(handler, attempt, {"kind": "invalid"})
    _check_selection(supplied=supplied, attempt=attempt)
    return {
        "handler": handler,
        "provider": provider,
        "failed": False,
        "chunks": [_chunk(chunk) for chunk in chunks],
        "post": _request(attempt.record),
        "post_call": attempt.logging.projection(),
        "error": None,
    }


async def execute(call: Input) -> Json:
    sync: list[Json] = []
    asynchronous: list[Json] = []
    for scenario in call.scenarios:
        sync.append(await _outcome(call, scenario, "sync"))
        asynchronous.append(await _outcome(call, scenario, "async"))
    return {"sync": sync, "async": asynchronous}


def run(argument: Json) -> Json:
    call = Input.model_validate(argument)
    return asyncio.run(execute(call))
