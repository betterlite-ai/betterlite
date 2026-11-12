import asyncio
import json
import os
from collections.abc import AsyncIterator, Awaitable, Iterator
from typing import cast

import httpx
import litellm
import openai
from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj
from litellm.llms.openai.common_utils import BaseOpenAILLM
from litellm.llms.openai.openai import OpenAIChatCompletion
from litellm.types.llms.openai import HttpxBinaryResponseContent
from typing_extensions import override

from .types import CallOutcome, ClientSpec, Fields, Input, Json, Reply, Request


class ReplyBody(httpx.AsyncByteStream, httpx.SyncByteStream):
    def __init__(self, chunks: list[bytes]) -> None:
        self.chunks = chunks

    @override
    def __iter__(self) -> Iterator[bytes]:
        yield from self.chunks

    @override
    async def __aiter__(self) -> AsyncIterator[bytes]:
        for chunk in self.chunks:
            yield chunk


class Upstream:
    def __init__(self, reply: Reply | None) -> None:
        self.reply = reply
        self.seen: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        if self.reply is None:
            raise httpx.ConnectError("the call needs no upstream", request=request)
        pairs = self.reply.get("headers")
        if pairs is None:
            pairs = []
        headers = [(pair[0], pair[1]) for pair in pairs]
        status = self.reply.get("status")
        if status is None:
            raise ValueError("a fixture reply needs a status")
        given = self.reply.get("chunks")
        if given is None:
            given = []
        chunks = [bytes(chunk) for chunk in given]
        text = self.reply.get("text")
        if text is not None:
            chunks.append(text.encode())
        return httpx.Response(status, headers=headers, stream=ReplyBody(chunks))


class PreCallLog:
    def __init__(self) -> None:
        self.input: Json = None
        self.api_base: Json = None
        self.body: Json = None

    def pre_call(self, **kwargs: Json) -> None:
        self.input = kwargs["input"]
        match kwargs["additional_args"]:
            case dict() as additional:
                self.api_base = additional.get("api_base")
                self.body = additional.get("complete_input_dict")
            case None | bool() | int() | float() | str() | list():
                raise ValueError("the logging additional args are an object")


def client_kind(client: openai.OpenAI | openai.AsyncOpenAI) -> str:
    match client:
        case openai.AsyncOpenAI():
            return "async"
        case openai.OpenAI():
            return "sync"


def timeout_seconds(timeout: float | httpx.Timeout | None) -> int:
    match timeout:
        case float() | int() as seconds:
            return int(seconds)
        case httpx.Timeout(read=float() as read):
            return int(read)
        case None | httpx.Timeout():
            return 0


def client_view(client: openai.OpenAI | openai.AsyncOpenAI, api_key: str | None) -> Fields:
    return {
        "kind": client_kind(client),
        "base_url": str(client.base_url),
        "timeout": timeout_seconds(client.timeout),
        "max_retries": client.max_retries,
        "organization": client.organization,
        "key_is_given": client.api_key == api_key,
    }


def supplied_client(
    spec: ClientSpec, transport: httpx.MockTransport
) -> openai.OpenAI | openai.AsyncOpenAI | None:
    supplied = spec.get("supplied")
    if supplied is None:
        return None
    if supplied == "async":
        return openai.AsyncOpenAI(
            api_key="sk-supplied",
            base_url="https://supplied.test/v1",
            organization="org-supplied",
            project="proj-supplied",
            timeout=45.0,
            max_retries=1,
            http_client=httpx.AsyncClient(transport=transport),
        )
    return openai.OpenAI(
        api_key="sk-supplied",
        base_url="https://supplied.test/v1",
        organization="org-supplied",
        project="proj-supplied",
        timeout=45.0,
        max_retries=1,
        http_client=httpx.Client(transport=transport),
    )


def built_client(spec: ClientSpec) -> openai.OpenAI | openai.AsyncOpenAI:
    params: Fields = {
        "is_async": spec.get("aspeech") is True,
        "api_key": spec.get("api_key"),
        "api_base": spec.get("api_base"),
        "timeout": float(spec["timeout"]),
        "max_retries": spec.get("max_retries"),
    }
    client = BaseOpenAILLM.get_cached_openai_client(params, "openai")  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L212-L213
    if client is None:
        raise ValueError("the speech call built no OpenAI client")
    return client


def optional_params(request: Request) -> Fields:
    options: Fields = {}
    response_format = request.get("response_format")
    if response_format is not None:
        options["response_format"] = response_format
    speed = request.get("speed")
    if speed is not None:
        options["speed"] = speed
    instructions = request.get("instructions")
    if instructions is not None:
        options["instructions"] = instructions
    extra_headers = request.get("extra_headers")
    if extra_headers is not None:
        options["extra_headers"] = extra_headers
    return options


async def drive(
    spec: ClientSpec,
    request: Request,
    log: PreCallLog,
    supplied: openai.OpenAI | openai.AsyncOpenAI | None,
) -> CallOutcome:
    api = OpenAIChatCompletion()
    response: HttpxBinaryResponseContent | None = None
    error: openai.APIStatusError | None = None
    try:
        outcome = api.audio_speech(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1556-L1579
            model=request["model"],
            input=request["input"],
            voice=request["voice"],
            optional_params=optional_params(request),
            api_key=spec.get("api_key"),
            api_base=spec.get("api_base"),
            organization=spec.get("organization"),
            project=spec.get("project"),
            max_retries=spec["max_retries"],
            timeout=float(spec["timeout"]),
            logging_obj=cast("LiteLLMLoggingObj", log),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1571
            aspeech=spec.get("aspeech"),
            client=supplied,
        )
        if spec.get("aspeech") is True:
            response = await cast("Awaitable[HttpxBinaryResponseContent]", outcome)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/openai.py#L1566-L1579
        else:
            response = outcome
    except openai.APIStatusError as caught:
        error = caught
    client = supplied if supplied is not None else built_client(spec)
    return response, error, client


def request_view(upstream: Upstream, request: Request) -> Json:
    if not upstream.seen:
        return None
    sent = upstream.seen[0]
    extra = request.get("extra_headers")
    if extra is None:
        extra = {}
    flat = dict(sent.headers)
    viewed: Fields = {name: flat[name] for name in extra}
    method = sent.method
    url = str(sent.url)
    accept = flat["accept"]
    authorization = flat["authorization"]
    organization = flat.get("openai-organization")
    project = flat.get("openai-project")
    body = cast("Json", json.loads(sent.content))  # https://github.com/python/typeshed/blob/289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad/stdlib/json/__init__.pyi#L79
    return {
        "method": method,
        "url": url,
        "accept": accept,
        "authorization": authorization,
        "organization": organization,
        "project": project,
        "extra_headers": viewed,
        "body": body,
    }


def outcome_view(
    response: HttpxBinaryResponseContent | None, error: openai.APIStatusError | None
) -> Json:
    if error is not None:
        return {
            "raised": {
                "status": error.status_code,
                "type": error.type,
                "param": error.param,
                "code": error.code,
            }
        }
    if response is None:
        raise ValueError("the call returned neither a response nor a status error")
    headers: Fields = {}
    headers.update(response.response.headers)
    content_type = headers.get("content-type")
    hidden = cast("Fields", response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L117-L123
    body: list[Json] = []
    body.extend(response.content)
    return {
        "status": response.response.status_code,
        "content_type": content_type,
        "headers": headers,
        "additional_headers": hidden.get("additional_headers"),
        "body": body,
    }


def run(input_value: Input) -> Json:
    spec = input_value["client"]
    request = input_value["request"]
    environment_organization = spec.get("environment_organization")
    if environment_organization is not None:
        os.environ["OPENAI_ORG_ID"] = environment_organization
    environment_project = spec.get("environment_project")
    if environment_project is not None:
        os.environ["OPENAI_PROJECT_ID"] = environment_project
    upstream = Upstream(input_value.get("reply"))
    supplied = supplied_client(spec, httpx.MockTransport(upstream.handle))
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(upstream.handle))
    litellm.aclient_session = httpx.AsyncClient(transport=httpx.MockTransport(upstream.handle))
    log = PreCallLog()
    response, error, client = asyncio.run(drive(spec, request, log, supplied))
    return {
        "client": client_view(client, spec.get("api_key")),
        "request": request_view(upstream, request),
        "result": outcome_view(response, error),
        "logging": {"input": log.input, "api_base": log.api_base, "body": log.body},
    }
