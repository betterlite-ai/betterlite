import asyncio
import socketserver
from collections.abc import AsyncGenerator, Awaitable, Callable, Sequence
from contextlib import asynccontextmanager
from json import dumps
from typing import Literal, Protocol, TypedDict, TypeVar
from urllib.parse import parse_qsl, urlsplit

from fastapi import HTTPException
from litellm import DualCache
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.types.utils import Choices, LLMResponseTypes, Message, ModelResponse, StreamingChoices
from pydantic import BaseModel, TypeAdapter
from pydantic import JsonValue as Json

T = TypeVar("T")
Returned = Exception | str | dict[str, Json] | None
PostCallHook = Callable[[dict[str, Json], UserAPIKeyAuth, LLMResponseTypes], Awaitable[LLMResponseTypes]]


class PreCallHooks(Protocol):
    async def async_pre_call_hook(
        self, user: UserAPIKeyAuth, cache: DualCache, data: dict[str, Json], call_type: Literal["completion"], /
    ) -> Returned: ...


class Answer(TypedDict):
    status: int
    headers: dict[str, str]
    body: Json


class Seen(TypedDict):
    path: str
    query: dict[str, str]
    headers: dict[str, str]
    body: Json


class HttpFacts(BaseModel):
    status_code: int
    detail: Json


class Scripted:
    def __init__(self, answers: Sequence[Answer], base_url: str) -> None:
        self.answers = list(answers)
        self.base_url = base_url
        self.seen: list[Seen] = []

    async def __call__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        head = (await reader.readuntil(b"\r\n\r\n")).decode("latin-1").split("\r\n")
        target = urlsplit(head[0].split(" ")[1])
        headers: dict[str, str] = {}
        for line in head[1:]:
            name, separator, value = line.partition(":")
            if separator:
                headers[name.strip().lower()] = value.strip()
        text = await reader.readexactly(int(headers["content-length"]))
        query = dict(parse_qsl(target.query))
        body = TypeAdapter(Json).validate_json(text)
        self.seen.append(Seen(path=target.path, query=query, headers=headers, body=body))
        answer = self.answers[len(self.seen) - 1]
        encoded = dumps(answer["body"]).encode()
        lines = [f"HTTP/1.1 {answer['status']} OK", f"Content-Length: {len(encoded)}"]
        lines.extend(f"{name}: {value}" for name, value in answer["headers"].items())
        writer.write("\r\n".join(lines).encode() + b"\r\n\r\n" + encoded)
        await writer.drain()
        writer.close()


@asynccontextmanager
async def scripted(answers: Sequence[Answer]) -> AsyncGenerator[Scripted]:
    # a listener nobody accepts from holds the connection open, so a call with no answer times out
    with socketserver.TCPServer(("127.0.0.1", 0), socketserver.BaseRequestHandler) as listener:
        server = Scripted(answers, f"http://127.0.0.1:{listener.server_address[1]}")
        if not answers:
            yield server
            return
        http = await asyncio.start_server(server, sock=listener.socket)
        try:
            yield server
        finally:
            http.close()


async def attempt(call: Awaitable[T]) -> T | BaseException:
    [outcome] = await asyncio.gather(call, return_exceptions=True)
    return outcome


async def post_call_contents(
    hook: PostCallHook, data: dict[str, Json], user: UserAPIKeyAuth, outputs: Sequence[str]
) -> dict[str, Json]:
    response = ModelResponse(choices=[Choices(message=Message(content=text, role="assistant")) for text in outputs])
    await hook(data, user, response)
    contents: list[Json] = []
    for choice in response.choices:
        match choice:
            case Choices():
                contents.append(choice.message.content)
            case StreamingChoices():
                raise TypeError("a streaming choice in a complete response")
    return {"contents": contents}


def failure_view(error: BaseException) -> dict[str, Json]:
    match error:
        case HTTPException():
            http = TypeAdapter(HttpFacts).validate_python(error, from_attributes=True)
            return {"status": http.status_code, "detail": http.detail, "code": None}
        case ProxyException():
            # the proxy turns ProxyException.openai_code into the string the error body carries
            code = None if error.openai_code is None else str(error.openai_code)
            return {"status": int(error.code), "detail": error.message, "code": code}
        case BaseException():
            return {"status": 500, "detail": str(error), "code": None}


def outcome_view(
    server: Scripted, outcome: dict[str, Json] | BaseException, names: Sequence[str], secrets: dict[str, str]
) -> Json:
    requests: list[Json] = []
    for call in server.seen:
        checked: dict[str, Json] = {name: call["headers"].get(name) == value for name, value in secrets.items()}
        shown: dict[str, Json] = {name: call["headers"][name] for name in names if name in call["headers"]}
        query: dict[str, Json] = {**call["query"]}
        view: dict[str, Json] = {"path": call["path"], "query": query, "headers": shown, "secrets": checked}
        requests.append({**view, "body": call["body"]})
    match outcome:
        case BaseException():
            return {"result": None, "error": failure_view(outcome), "requests": requests}
        case dict():
            return {"result": outcome, "error": None, "requests": requests}
