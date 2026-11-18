import asyncio
import os
from collections.abc import AsyncIterator, Awaitable, Iterator
from contextlib import AsyncExitStack
from dataclasses import dataclass, field
from http import HTTPStatus
from typing import Protocol, TypedDict, Unpack, runtime_checkable
from unittest.mock import MagicMock

import httpx
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.bedrock.chat.converse_handler import BedrockConverseLLM
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import ModelResponse, ModelResponseStream
from openai import APIError
from pydantic import BaseModel, ConfigDict, TypeAdapter
from pydantic import JsonValue as Json
from typing_extensions import override


class Input(TypedDict):
    model: str
    params: dict[str, Json]
    headers: dict[str, str]
    bearer: str | None
    asynchronous: bool
    status: int
    body: str
    frames: list[list[int]]
    consume: bool


class CallArgs(TypedDict):
    model: str
    messages: list[dict[str, str]]
    api_base: str
    custom_prompt_dict: dict[str, Json]
    model_response: ModelResponse
    encoding: None
    logging_obj: MagicMock
    optional_params: dict[str, Json]
    acompletion: bool
    timeout: int
    litellm_params: dict[str, Json]
    extra_headers: dict[str, str]
    client: HTTPHandler | AsyncHTTPHandler
    api_key: str | None


@runtime_checkable
class SyncCall(Protocol):
    def completion(self, **kwargs: Unpack[CallArgs]) -> ModelResponse | CustomStreamWrapper: ...


@runtime_checkable
class AsyncCall(Protocol):
    def completion(
        self, **kwargs: Unpack[CallArgs]
    ) -> Awaitable[ModelResponse | CustomStreamWrapper]: ...


class Bound(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    sync: SyncCall
    asynchronous: AsyncCall


class Failure(BaseModel):
    status_code: int


class Message(TypedDict):
    content: str | None


class Choice(TypedDict):
    message: Message


class Reply(TypedDict):
    choices: list[Choice]


class Delta(TypedDict):
    content: str | None


class DeltaChoice(TypedDict):
    delta: Delta


class Chunk(TypedDict):
    choices: list[DeltaChoice]


@dataclass
class Fixture:
    input: Input
    url: str = ""
    body: str = ""
    headers: dict[str, str] = field(default_factory=dict)
    reads: int = 0
    chunks: int = 0

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.url = str(request.url)
        self.body = request.content.decode()
        self.headers = dict(request.headers)
        return httpx.Response(
            self.input["status"],
            headers={"x-request-id": "request-1"},
            stream=Body(self),
        )

    def arguments(self, client: HTTPHandler | AsyncHTTPHandler) -> CallArgs:
        details: dict[str, Json] = {}
        params: dict[str, Json] = {}
        messages: list[Json] = []
        return CallArgs(
            model=self.input["model"],
            messages=[{"role": "user", "content": "hello"}],
            api_base="https://bedrock-runtime.us-west-2.amazonaws.com",
            custom_prompt_dict={},
            model_response=ModelResponse(id="fixture", created=0),
            encoding=None,
            logging_obj=MagicMock(
                model_call_details=details,
                litellm_params=params,
                stream_options=None,
                messages=messages,
            ),
            optional_params=dict(self.input["params"]),
            acompletion=self.input["asynchronous"],
            timeout=3,
            litellm_params={},
            extra_headers=self.input["headers"],
            client=client,
            api_key=self.input["bearer"],
        )

    async def execute(self, environment: dict[str, str]) -> dict[str, Json]:
        transport = httpx.MockTransport(self.respond)
        saved = os.environ.copy()
        try:
            os.environ.clear()
            os.environ.update(environment)
            async with AsyncExitStack() as stack:
                if self.input["asynchronous"]:
                    handler = AsyncHTTPHandler(transport=transport)
                    await stack.enter_async_context(handler.client)
                    return await observe(self, handler)
                client = stack.enter_context(httpx.Client(transport=transport))
                return await observe(self, HTTPHandler(client=client))
        finally:
            os.environ.clear()
            os.environ.update(saved)


class Body(httpx.SyncByteStream, httpx.AsyncByteStream):
    def __init__(self, fixture: Fixture) -> None:
        self.fixture = fixture

    @override
    def __iter__(self) -> Iterator[bytes]:
        if (
            not self.fixture.url.endswith("/converse-stream")
            or self.fixture.input["status"] != HTTPStatus.OK
        ):
            self.fixture.reads += 1
            yield self.fixture.input["body"].encode()
            return
        for frame in self.fixture.input["frames"]:
            self.fixture.reads += 1
            yield bytes(frame)

    @override
    async def __aiter__(self) -> AsyncIterator[bytes]:
        for chunk in self:
            yield chunk


async def invoke(
    fixture: Fixture, client: HTTPHandler | AsyncHTTPHandler
) -> ModelResponse | CustomStreamWrapper:
    handler = BedrockConverseLLM()
    handlers: dict[str, BedrockConverseLLM] = {"sync": handler, "asynchronous": handler}
    bound = Bound.model_validate(handlers)
    if fixture.input["asynchronous"]:
        return await bound.asynchronous.completion(**fixture.arguments(client))
    return bound.sync.completion(**fixture.arguments(client))


async def stream_items(
    wrapper: CustomStreamWrapper, input_value: Input
) -> AsyncIterator[ModelResponseStream]:
    if input_value["asynchronous"]:
        async for chunk in wrapper:
            yield chunk
    else:
        for chunk in wrapper:
            yield chunk


async def consume_stream(wrapper: CustomStreamWrapper, fixture: Fixture) -> None:
    if fixture.input["consume"]:
        async for chunk in stream_items(wrapper, fixture.input):
            projected = TypeAdapter(Chunk).validate_json(chunk.model_dump_json())
            if any(choice["delta"]["content"] for choice in projected["choices"]):
                fixture.chunks += 1


async def observe(fixture: Fixture, client: HTTPHandler | AsyncHTTPHandler) -> dict[str, Json]:
    content: str | None = None
    status: int | None = None
    streaming = False
    try:
        result = await invoke(fixture, client)
        streaming = fixture.input["params"].get("stream") is True
        if streaming:
            wrapper = TypeAdapter(
                CustomStreamWrapper, config=ConfigDict(arbitrary_types_allowed=True)
            ).validate_python(result)
            await consume_stream(wrapper, fixture)
        else:
            response = TypeAdapter(ModelResponse).validate_python(result)
            reply = TypeAdapter(Reply).validate_json(response.model_dump_json())
            content = reply["choices"][0]["message"]["content"]
    except BedrockError as error:
        status = error.status_code
    except APIError as error:
        status = Failure.model_validate(error, from_attributes=True).status_code
    headers: dict[str, Json] = {
        name.lower(): ("sigv4" if value.startswith("AWS4-HMAC-SHA256") else value)
        for name, value in fixture.headers.items()
        if name.lower() in {"authorization", "content-type", "anthropic-beta", "x-custom"}
    }
    return {
        "url": fixture.url,
        "body": TypeAdapter(Json).validate_json(fixture.body),
        "headers": headers,
        "status": status,
        "content": content,
        "stream": streaming,
        "chunks": fixture.chunks,
    }


def run(input_value: Input) -> Json:
    environment = {
        "AWS_ACCESS_KEY_ID": os.environ["AWS_ACCESS_KEY_ID"],
        "AWS_SECRET_ACCESS_KEY": os.environ["AWS_SECRET_ACCESS_KEY"],
        "AWS_REGION_NAME": "us-west-2",
        "AWS_EC2_METADATA_DISABLED": "true",
        "AWS_CONFIG_FILE": os.devnull,
        "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
        "LITELLM_RUST": "false",
        "LITELLM_LOCAL_MODEL_COST_MAP": "true",
    }
    return asyncio.run(Fixture(input_value).execute(environment))
