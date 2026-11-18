import asyncio
import os
from dataclasses import dataclass, field
from datetime import UTC, datetime
from functools import partial
from typing import Protocol

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L51-L62
import httpx
from aiohttp import ClientSession
from litellm.exceptions import Timeout
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.embed import embedding as embedding_module
from litellm.llms.cohere.embed import handler as cohere_handler
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.types.utils import EmbeddingResponse, LlmProviders
from pydantic import JsonValue
from typing_extensions import override

from .wire import CASE, Case, Reply, request_value, response_value, settle, timeout_fields


class Calls(Protocol):
    phases: list[JsonValue]

    def pre_call(self, **_arguments: JsonValue) -> None: ...

    def post_call(self, **_arguments: JsonValue) -> None: ...


class Logger(Calls):
    def __init__(self) -> None:
        self.phases: list[JsonValue] = []

    @override
    def pre_call(self, **_arguments: JsonValue) -> None:
        self.phases.append("pre_api_call")

    @override
    def post_call(self, **_arguments: JsonValue) -> None:
        self.phases.append("post_api_call")


class Tokenizer(Protocol):
    def encode(self, text: str, /) -> list[int]: ...


class FixedTokenizer(Tokenizer):
    @override
    def encode(self, text: str, /) -> list[int]:
        return [ord(char) for char in text]


@dataclass
class Fixture:
    replies: list[Reply]
    now: int
    requests: list[JsonValue] = field(default_factory=list)
    factory: list[JsonValue] = field(default_factory=list)

    def answer(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request_value(request))
        if not self.replies:
            raise httpx.ReadError("the reply queue is empty")
        reply = self.replies.pop(0)
        failure = reply.get("failure")
        if failure == "timeout":
            raise httpx.ReadTimeout("read timed out")
        if failure == "failed":
            raise httpx.ReadError("the transport failed")
        return httpx.Response(
            reply["status"], headers=reply.get("headers"), content=reply["body"].encode()
        )

    def client(self) -> HTTPHandler:
        transport = httpx.MockTransport(self.answer)
        return HTTPHandler(client=httpx.Client(transport=transport, follow_redirects=True))

    def sync_client(self, params: dict[str, JsonValue] | None = None) -> HTTPHandler:
        timeout: JsonValue = None if params is None else params.get("timeout")
        record: dict[str, JsonValue] = {
            "kind": "sync",
            "provider": None,
            "timeout": timeout_fields(timeout),
        }
        self.factory.append(record)
        return self.client()

    def async_client(
        self,
        llm_provider: str,
        params: dict[str, JsonValue] | None = None,
        shared_session: ClientSession | None = None,  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L1713
    ) -> AsyncHTTPHandler:
        timeout: JsonValue = None if params is None else params.get("timeout")
        record: dict[str, JsonValue] = {
            "kind": "async",
            "provider": LlmProviders(llm_provider).value,
            "timeout": timeout_fields(timeout),
        }
        self.factory.append(record)
        return AsyncHTTPHandler(transport=httpx.MockTransport(self.answer))


def configure(fixture: Fixture) -> None:
    embedding_module._get_httpx_client = fixture.sync_client  # type: ignore[attr-defined]  # noqa: SLF001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L125
    embedding_module.get_async_httpx_client = fixture.async_client  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L158
    cohere_handler.get_async_httpx_client = fixture.async_client  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cohere/embed/handler.py#L84
    botocore.auth.get_current_datetime = partial(datetime.fromtimestamp, fixture.now, tz=UTC)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L60


def set_environment(case: Case) -> None:
    environment = case.get("environment")
    os.environ.clear()
    if environment is not None:
        os.environ.update(environment)


def supplied(case: Case, fixture: Fixture) -> HTTPHandler | AsyncHTTPHandler | None:
    kind = case.get("supplied")
    if kind == "sync":
        return fixture.client()
    if kind == "async":
        return AsyncHTTPHandler(transport=httpx.MockTransport(fixture.answer))
    return None


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = CASE.validate_python(input_value)
    set_environment(case)
    raw = case.get("replies")
    fixture = Fixture([] if raw is None else list(raw), case["now"])
    configure(fixture)
    values = case.get("optional_params")
    optional_params: dict[str, JsonValue] = {} if values is None else dict(values)
    litellm_params: dict[str, JsonValue] = {"drop_params": case.get("drop_params")}
    logger = Logger()
    ok = False
    status: int | None = None
    response: JsonValue = None
    try:
        pending = embedding_module.BedrockEmbedding().embeddings(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L65
            model=case["model"],
            input=list(case["input"]),
            api_base=case.get("api_base"),
            model_response=EmbeddingResponse(),
            print_verbose=print,
            encoding=FixedTokenizer(),
            logging_obj=logger,
            client=supplied(case, fixture),
            timeout=case.get("timeout"),
            aembedding=case.get("aembedding"),
            extra_headers=case.get("extra_headers"),
            optional_params=optional_params,
            litellm_params=litellm_params,
            api_key=case.get("api_key"),
        )
        reply = asyncio.run(settle(pending))
    except BedrockError as error:
        status = error.status_code
    except Timeout:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L356
        status = 408
    except httpx.TimeoutException:
        status = 408
    except httpx.HTTPStatusError as error:
        status = error.response.status_code
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/cohere/embed/handler.py#L95
        status = None
    else:
        ok = True
        response = response_value(reply)
    return {
        "ok": ok,
        "status": status,
        "type": None,
        "param": None,
        "code": None,
        "requests": fixture.requests,
        "factory": fixture.factory,
        "response": response,
        "phases": logger.phases,
        "remaining": optional_params,
    }
