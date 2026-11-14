import asyncio
import json
from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from typing import Literal, Protocol, TypeAlias, TypedDict, TypeVar, runtime_checkable

import httpx
import litellm
from litellm.a2a_protocol.providers.bedrock_agentcore import config as agentcore_config
from litellm.a2a_protocol.providers.bedrock_agentcore import handler as agentcore_handler
from litellm.a2a_protocol.providers.bedrock_agentcore import transformation as agentcore_transform
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.llms.custom_http import httpxSpecialProvider
from pydantic import JsonValue, TypeAdapter

DECODER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
CLIENT_KEY = "async_httpx_client"
Outcome = TypeVar("Outcome")


@runtime_checkable
class LineSource(Protocol):
    def aiter_lines(self) -> AsyncIterator[str]: ...


@runtime_checkable
class Transformation(Protocol):
    def get_url_and_signed_request(
        self,
        request_id: str,
        params: Mapping[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        *,
        method: str = "message/send",
        agent_extra_headers: dict[str, str] | None = None,
    ) -> tuple[str, dict[str, str], bytes]: ...

    def parse_sse_events(self, response: LineSource) -> AsyncIterator[JsonValue]: ...


@runtime_checkable
class AgentHandler(Protocol):
    async def handle_non_streaming(
        self,
        request_id: str,
        params: Mapping[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        agent_extra_headers: dict[str, str] | None,
    ) -> dict[str, JsonValue]: ...

    def handle_streaming(
        self,
        request_id: str,
        params: Mapping[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        agent_extra_headers: dict[str, str] | None,
    ) -> AsyncIterator[dict[str, JsonValue]]: ...


@runtime_checkable
class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler, *, ttl: int) -> None: ...


@runtime_checkable
class AgentConfig(Protocol):
    async def handle_non_streaming(
        self,
        request_id: str,
        params: dict[str, JsonValue],
        api_base: str | None = None,
        **kwargs: JsonValue,
    ) -> dict[str, JsonValue]: ...

    def handle_streaming(
        self,
        request_id: str,
        params: dict[str, JsonValue],
        api_base: str | None = None,
        **kwargs: JsonValue,
    ) -> AsyncIterator[dict[str, JsonValue]]: ...


TRANSFORM: Callable[[], Transformation] = TypeAdapter(Callable[[], Transformation]).validate_python(
    agentcore_transform.BedrockAgentCoreA2ATransformation
)
HANDLER: Callable[[], AgentHandler] = TypeAdapter(Callable[[], AgentHandler]).validate_python(
    agentcore_handler.BedrockAgentCoreA2AHandler
)
CONFIG: Callable[[], AgentConfig] = TypeAdapter(Callable[[], AgentConfig]).validate_python(
    agentcore_config.BedrockAgentCoreA2AConfig
)


class Answer(TypedDict):
    status: int
    body: JsonValue


class StreamAnswer(TypedDict):
    status: int
    content_type: str
    lines: list[str]


class SignedRequestInput(TypedDict):
    function: Literal["signed_request"]
    request_id: str
    params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    agent_extra_headers: dict[str, str] | None


class ParseSseInput(TypedDict):
    function: Literal["parse_sse"]
    lines: list[str]


class NonStreamingInput(TypedDict):
    function: Literal["non_streaming"]
    request_id: str
    params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    agent_extra_headers: dict[str, str] | None
    answer: Answer


class StreamingInput(TypedDict):
    function: Literal["streaming"]
    request_id: str
    params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    agent_extra_headers: dict[str, str] | None
    answer: StreamAnswer


class ConfigInput(TypedDict):
    function: Literal["config"]
    request_id: str
    params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue] | None
    answer: Answer
    stream_answer: StreamAnswer


Input: TypeAlias = (
    SignedRequestInput | ParseSseInput | NonStreamingInput | StreamingInput | ConfigInput
)


class Lines:
    def __init__(self, lines: list[str]) -> None:
        self.lines = lines

    async def aiter_lines(self) -> AsyncIterator[str]:
        for line in self.lines:
            yield line


class Reply:
    def __init__(self, status: int, headers: dict[str, str], content: bytes) -> None:
        self.status = status
        self.headers = headers
        self.content = content

    def __call__(self, _request: httpx.Request) -> httpx.Response:
        return httpx.Response(self.status, headers=self.headers, content=self.content)


def install_client(reply: Reply) -> None:
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    cache = TypeAdapter(ClientCache).validate_python(holder)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(reply))
    cache.set_cache(key=CLIENT_KEY + httpxSpecialProvider.A2AProvider, value=client, ttl=600)


def json_reply(answer: Answer) -> Reply:
    headers = {"content-type": "application/json"}
    return Reply(answer["status"], headers, json.dumps(answer["body"]).encode())


def stream_reply(answer: StreamAnswer) -> Reply:
    headers = {"content-type": answer["content_type"]}
    body = "".join(f"{line}\n" for line in answer["lines"])
    return Reply(answer["status"], headers, body.encode())


async def collected(events: AsyncIterator[JsonValue]) -> list[JsonValue]:
    values: list[JsonValue] = [event async for event in events]
    return values


def signed_request(input_value: SignedRequestInput) -> JsonValue:
    transform = TRANSFORM()
    url, headers, body = transform.get_url_and_signed_request(
        input_value["request_id"],
        input_value["params"],
        input_value["litellm_params"],
        method="message/send",
        agent_extra_headers=input_value["agent_extra_headers"],
    )
    output: dict[str, JsonValue] = {
        "url": url,
        "headers": DECODER.validate_python(headers),
        "body": DECODER.validate_json(body),
    }
    return output


def parse_sse(input_value: ParseSseInput) -> JsonValue:
    transform = TRANSFORM()
    events = transform.parse_sse_events(Lines(input_value["lines"]))
    values = asyncio.run(collected(events))
    output: dict[str, JsonValue] = {"events": values}
    return output


async def answered(reply: Reply, response: Awaitable[Outcome]) -> Outcome:
    install_client(reply)
    return await response


def config(input_value: ConfigInput) -> JsonValue:
    instance = CONFIG()
    output: dict[str, JsonValue] = {}
    try:
        value = asyncio.run(
            answered(
                json_reply(input_value["answer"]),
                instance.handle_non_streaming(
                    input_value["request_id"],
                    input_value["params"],
                    litellm_params=input_value["litellm_params"],
                ),
            )
        )
        output["non_streaming"] = {"ok": True, "value": value}
    except ValueError:
        output["non_streaming"] = {"ok": False}
    try:
        events = asyncio.run(
            answered(
                stream_reply(input_value["stream_answer"]),
                collected(
                    instance.handle_streaming(
                        input_value["request_id"],
                        input_value["params"],
                        litellm_params=input_value["litellm_params"],
                    )
                ),
            )
        )
        output["streaming"] = {"ok": True, "value": events}
    except ValueError:
        output["streaming"] = {"ok": False}
    return output


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "signed_request":
        return signed_request(input_value)
    if input_value["function"] == "parse_sse":
        return parse_sse(input_value)
    if input_value["function"] == "non_streaming":
        return asyncio.run(
            answered(
                json_reply(input_value["answer"]),
                HANDLER().handle_non_streaming(
                    input_value["request_id"],
                    input_value["params"],
                    input_value["litellm_params"],
                    input_value["agent_extra_headers"],
                ),
            )
        )
    if input_value["function"] == "streaming":
        events = asyncio.run(
            answered(
                stream_reply(input_value["answer"]),
                collected(
                    HANDLER().handle_streaming(
                        input_value["request_id"],
                        input_value["params"],
                        input_value["litellm_params"],
                        input_value["agent_extra_headers"],
                    )
                ),
            )
        )
        output: dict[str, JsonValue] = {"events": events}
        return output
    return config(input_value)
