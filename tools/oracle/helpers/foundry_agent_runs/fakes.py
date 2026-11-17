import asyncio
import time
from collections.abc import AsyncIterator
from typing import Final, Protocol

import httpx
import litellm
import litellm.llms.custom_httpx.http_handler
import litellm.utils
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from pydantic import JsonValue
from typing_extensions import override

from .types import JSON, JsonObject, Step

COUNT_MODEL: Final = "gpt-3.5-turbo"


class MissingStepError(RuntimeError):
    pass


class SleepSeam(Protocol):
    def sleep(self, seconds: float) -> None: ...

    async def asleep(self, delay: float) -> None: ...


class BodyStream(httpx.AsyncByteStream):
    def __init__(self, body: str) -> None:
        self.body = body

    @override
    async def __aiter__(self) -> AsyncIterator[bytes]:
        yield self.body.encode()


def sent(method: str, url: str, headers: JsonValue) -> JsonObject:
    return {
        "method": method,
        "url": url,
        "headers": headers,
        "timeout": None,
    }


class Recorder:
    def __init__(self, steps: list[Step]) -> None:
        self.steps = list(steps)
        self.requests: list[JsonObject] = []

    def take(
        self, request: JsonObject, data: str | None, *, stream: bool
    ) -> httpx.Response:
        request["body"] = None if data is None else JSON.validate_json(data)
        self.requests.append(request)
        if not self.steps:
            raise MissingStepError("the fixture has no scripted step")
        step = self.steps.pop(0)
        if stream:
            return httpx.Response(step["status"], stream=BodyStream(step["body"]))
        return httpx.Response(step["status"], content=step["body"].encode())


class Scripted:
    def __init__(self, recorder: Recorder) -> None:
        self.recorder = recorder


class SyncClient(Scripted):
    def get(self, *, url: str, headers: JsonValue = None) -> httpx.Response:
        return self.recorder.take(sent("GET", url, headers), None, stream=False)

    def post(
        self, *, url: str, headers: JsonValue = None, data: str | None = None
    ) -> httpx.Response:
        return self.recorder.take(sent("POST", url, headers), data, stream=False)


class AsyncClient(Scripted):
    async def get(self, *, url: str, headers: JsonValue = None) -> httpx.Response:
        return self.recorder.take(sent("GET", url, headers), None, stream=False)

    async def post(
        self,
        *,
        url: str,
        headers: JsonValue = None,
        data: str | None = None,
        stream: bool = False,
    ) -> httpx.Response:
        return self.recorder.take(sent("POST", url, headers), data, stream=stream)


class Factory:
    def __init__(self, steps: list[Step]) -> None:
        self.recorder = Recorder(steps)
        self.kind = ""
        self.ssl_verify: JsonValue = None

    def sync_client(self, params: JsonObject | None = None) -> SyncClient:
        self.kind = "sync"
        self.read_ssl(params)
        return SyncClient(self.recorder)

    def async_client(
        self, llm_provider: LlmProviders, params: JsonObject | None = None
    ) -> AsyncClient:
        if llm_provider is not LlmProviders.AZURE_AI:
            raise ValueError("the fixture client only serves Azure AI agents")
        self.kind = "async"
        self.read_ssl(params)
        return AsyncClient(self.recorder)

    def read_ssl(self, params: JsonObject | None) -> None:
        if params is not None:
            self.ssl_verify = params.get("ssl_verify")


class Clock(SleepSeam):
    def __init__(self) -> None:
        self.waits: list[float] = []

    @override
    def sleep(self, seconds: float) -> None:
        self.waits.append(seconds)

    @override
    async def asleep(self, delay: float) -> None:
        self.waits.append(delay)


class Resolver:
    def __init__(self, token: str | None) -> None:
        self.token = token
        self.seen: list[JsonObject] = []

    def __call__(self, params: GenericLiteLLMParams) -> str | None:
        self.seen.append(
            {
                "azure_scope": params.azure_scope,
                "tenant_id": params.tenant_id,
                "client_id": params.client_id,
                "client_secret": params.client_secret,
                "azure_ad_token": params.azure_ad_token,
                "api_key": params.api_key,
            }
        )
        return self.token


def count_tokens(
    *,
    model: str,
    messages: list[JsonObject] | None = None,
    text: str | None = None,
    count_response_tokens: bool = False,
) -> int:
    if model != COUNT_MODEL:
        raise ValueError("the handler counts tokens with gpt-3.5-turbo")
    if messages is not None:
        return len(messages)
    if text is not None and count_response_tokens:
        return len(text)
    return 0


def install_fixtures(clock: Clock, factory: Factory) -> None:
    time.sleep = clock.sleep  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L411
    asyncio.sleep = clock.asleep  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L539
    litellm.llms.custom_httpx.http_handler._get_httpx_client = factory.sync_client  # type: ignore[assignment]  # noqa: SLF001  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue, reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L317-L320
    litellm.llms.custom_httpx.http_handler.get_async_httpx_client = factory.async_client  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L444-L448
    litellm.utils.token_counter = count_tokens  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L240-L243
