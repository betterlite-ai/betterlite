import time
import uuid
from collections.abc import AsyncIterator
from uuid import UUID

import httpx
import litellm
import litellm.llms.custom_httpx.http_handler
from pydantic import JsonValue

from .types import JSON, JsonObject


class FixedUuid:
    def __init__(self, chunk_id: str) -> None:
        short = chunk_id.removeprefix("chatcmpl-")
        self.value = UUID(hex=f"{short}-0000-4000-8000-000000000000")

    def __call__(self) -> UUID:
        return self.value


class FixedTime:
    def __init__(self, created: int) -> None:
        self.created = created

    def __call__(self) -> float:
        return float(self.created)


class FixedStream(httpx.AsyncByteStream):
    def __init__(self, body: str, split: int | None) -> None:
        self.body = body
        self.split = split

    async def __aiter__(self) -> AsyncIterator[bytes]:
        encoded = self.body.encode()
        if self.split is None:
            yield encoded
            return
        yield encoded[: self.split]
        yield encoded[self.split :]


class FakeClient:
    def __init__(self, status: int, body: str, split: int | None = None) -> None:
        self.status = status
        self.body = body
        self.split = split
        self.url = ""
        self.headers: JsonValue = None
        self.payload: JsonValue = None
        self.streaming = False

    async def post(
        self,
        url: str,
        *,
        headers: JsonValue = None,
        data: str = "",
        stream: bool = False,
    ) -> httpx.Response:
        self.url = url
        self.headers = headers
        self.payload = JSON.validate_json(data)
        self.streaming = stream
        return httpx.Response(
            self.status,
            stream=FixedStream(self.body, self.split),
            request=httpx.Request("POST", url),
        )


class ClientFactory:
    def __init__(self, client: FakeClient) -> None:
        self.client = client
        self.ssl_verify: JsonValue = None

    def __call__(
        self, *, llm_provider: litellm.LlmProviders, params: JsonObject | None = None
    ) -> FakeClient:
        if llm_provider is not litellm.LlmProviders.AZURE_AI:
            raise ValueError("the fixture client only serves Azure AI agents")
        if params is not None:
            self.ssl_verify = params.get("ssl_verify")
        return self.client


class Recorder:
    def __init__(self, *, fail: bool) -> None:
        self.fail = fail
        self.calls: list[JsonObject] = []

    def count(
        self,
        *,
        model: str,
        messages: list[JsonObject] | None = None,
        text: str | None = None,
        count_response_tokens: bool = False,
    ) -> int:
        self.calls.append(
            {
                "model": model,
                "messages": JSON.validate_python(messages),
                "text": text,
                "count_response_tokens": count_response_tokens,
            }
        )
        if self.fail:
            raise ValueError("the fixture counter cannot count")
        if messages is not None:
            return len(messages)
        if text is not None:
            return len(text)
        return 0


def patch_identity(chunk_id: str, created: int) -> None:
    uuid.uuid4 = FixedUuid(chunk_id)
    time.time = FixedTime(created)


def install_client(client: FakeClient) -> ClientFactory:
    factory = ClientFactory(client)
    litellm.llms.custom_httpx.http_handler.get_async_httpx_client = factory  # noqa: E501  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L589-L592
    return factory
