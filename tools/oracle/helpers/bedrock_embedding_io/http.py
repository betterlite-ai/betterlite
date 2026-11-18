import asyncio
from collections.abc import Callable, Coroutine
from dataclasses import dataclass, field
from functools import partial
from typing import Literal, cast

import httpx
from aiohttp import ClientSession
from litellm.exceptions import Timeout
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.embed import embedding as embedding_module
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import LlmProviders
from pydantic import BaseModel, ConfigDict, JsonValue

from .types import HttpRequest
from .views import BODY, failure_value, set_environment

Client = HTTPHandler | AsyncHTTPHandler | None

SyncCall = Callable[
    [Client, float | httpx.Timeout | None, str, dict[str, str], dict[str, JsonValue]],
    dict[str, JsonValue],
]
AsyncCall = Callable[
    [Client, float | httpx.Timeout | None, str, dict[str, str], dict[str, JsonValue]],
    Coroutine[None, None, dict[str, JsonValue]],
]


class Caller(embedding_module.BedrockEmbedding):
    def sync(
        self,
        client: Client,
        timeout: float | httpx.Timeout | None,
        url: str,
        headers: dict[str, str],
        data: dict[str, JsonValue],
    ) -> dict[str, JsonValue]:
        call = cast(SyncCall, self._make_sync_call)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L111-L142
        return call(client, timeout, url, headers, data)

    def asynchronous(
        self,
        client: Client,
        timeout: float | httpx.Timeout | None,
        url: str,
        headers: dict[str, str],
        data: dict[str, JsonValue],
    ) -> dict[str, JsonValue]:
        call = cast(AsyncCall, self._make_async_call)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L144-L176
        return asyncio.run(call(client, timeout, url, headers, data))


@dataclass
class FactoryCall:
    kind: Literal["sync", "async"]
    provider: str | None
    timeout: list[JsonValue] | None


@dataclass
class Fixture:
    case: HttpRequest
    factory_calls: list[FactoryCall] = field(default_factory=list)
    supplied: list[JsonValue] = field(default_factory=list)
    factory: list[JsonValue] = field(default_factory=list)
    requests: int = 0

    def respond(
        self, recorder: list[JsonValue], request: httpx.Request
    ) -> httpx.Response:
        self.requests += 1
        if self.requests == 1:
            recorder.append(request_value(self.case, request))
        reply = self.case["reply"]
        redirect = reply.get("redirect")
        if redirect is not None and self.requests > 1:
            return httpx.Response(
                redirect["status"],
                headers=redirect["headers"],
                content=redirect["body"].encode(),
            )
        failure = reply.get("failure")
        if failure == "timeout":
            raise httpx.ReadTimeout("read timed out")
        if failure == "failed":
            raise httpx.ReadError("the transport failed")
        return httpx.Response(
            reply["status"], headers=reply["headers"], content=reply["body"].encode()
        )

    def sync_client(self, params: dict[str, JsonValue] | None = None) -> HTTPHandler:
        timeout = None if params is None else params.get("timeout")
        self.factory_calls.append(FactoryCall("sync", None, timeout_fields(timeout)))
        return HTTPHandler(
            client=httpx.Client(
                transport=httpx.MockTransport(partial(self.respond, self.factory)),
                follow_redirects=True,
            )
        )

    def async_client(
        self,
        llm_provider: str,
        params: dict[str, JsonValue] | None = None,
        shared_session: ClientSession | None = None,  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L158
    ) -> AsyncHTTPHandler:
        timeout = None if params is None else params.get("timeout")
        provider = LlmProviders(llm_provider).value
        self.factory_calls.append(FactoryCall("async", provider, timeout_fields(timeout)))
        return AsyncHTTPHandler(
            transport=httpx.MockTransport(partial(self.respond, self.factory))
        )


class TimeoutView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)
    connect: float
    read: float
    write: float
    pool: float


def timeout_fields(timeout: JsonValue) -> list[JsonValue] | None:
    if timeout is None:
        return None
    value = TimeoutView.model_validate(timeout)
    return [value.connect, value.read, value.write, value.pool]


def request_value(case: HttpRequest, request: httpx.Request) -> dict[str, JsonValue]:
    names = {name.lower() for name in case["headers"]}
    headers: dict[str, JsonValue] = {
        name.lower(): value
        for name, value in dict(request.headers).items()
        if name.lower() in names
    }
    return {
        "url": str(request.url),
        "headers": headers,
        "data": BODY.validate_json(request.content),
    }


def chosen_client(case: HttpRequest, fixture: Fixture) -> Client:
    transport = httpx.MockTransport(partial(fixture.respond, fixture.supplied))
    supplied = case["supplied"]
    if supplied == "sync":
        return HTTPHandler(client=httpx.Client(transport=transport, follow_redirects=True))
    if supplied == "async":
        return AsyncHTTPHandler(transport=transport)
    return None


def input_timeout(case: HttpRequest) -> float | httpx.Timeout | None:
    record = case.get("timeout_record")
    if record is not None:
        return httpx.Timeout(
            connect=record["connect"],
            read=record["read"],
            write=record["write"],
            pool=record["pool"],
        )
    return case.get("timeout")


def configure(fixture: Fixture) -> None:
    set_environment({})
    embedding_module._get_httpx_client = fixture.sync_client  # type: ignore[attr-defined]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L112
    embedding_module.get_async_httpx_client = fixture.async_client  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L157


def run_call(case: HttpRequest, fixture: Fixture) -> dict[str, JsonValue]:
    caller = Caller()
    client = chosen_client(case, fixture)
    timeout = input_timeout(case)
    if case["asynchronous"]:
        return caller.asynchronous(
            client, timeout, case["url"], case["headers"], case["data"]
        )
    return caller.sync(client, timeout, case["url"], case["headers"], case["data"])


def provider_json_response(case: HttpRequest) -> JsonValue:
    fixture = Fixture(case)
    configure(fixture)
    value = run_call(case, fixture)
    factory: list[JsonValue] = []
    for call in fixture.factory_calls:
        record: dict[str, JsonValue] = {
            "kind": call.kind,
            "provider": call.provider,
            "timeout": call.timeout,
        }
        factory.append(record)
    return {
        "factory": factory,
        "supplied_requests": fixture.supplied,
        "factory_requests": fixture.factory,
        "value": value,
    }


def http_error(case: HttpRequest) -> JsonValue:
    fixture = Fixture(case)
    configure(fixture)
    try:
        value = run_call(case, fixture)
    except Timeout as error:
        if case["asynchronous"]:
            message, separator, _elapsed = str(error).partition(", time taken=")
            if separator:
                error.message = f"{message}, time taken=<seconds> seconds"
        raise
    except BedrockError as error:
        names = {name.lower() for name in case["reply"]["headers"]}
        redirect = case["reply"].get("redirect")
        if redirect is not None:
            names.update(name.lower() for name in redirect["headers"])
        return failure_value(error, names)
    return {"kind": "ok", "value": value}
