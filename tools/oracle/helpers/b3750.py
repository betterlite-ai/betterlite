import asyncio
import socketserver
from collections.abc import AsyncGenerator, AsyncIterator, Sequence
from contextlib import asynccontextmanager
from json import dumps
from typing import NotRequired, Protocol, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.aim.aim import (
    AimGuardrail,
    AimGuardrailMissingSecrets,
    AimOutputGuardrailResult,
)
from litellm.proxy.proxy_server import StreamingCallbackError
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import Delta, ModelResponseStream, StreamingChoices
from pydantic import BaseModel, TypeAdapter
from pydantic import JsonValue as Json
from websockets.asyncio.server import ServerConnection, serve

HTTP_BAD_REQUEST = 400
ANALYZE_PATH = "/fw/v1/analyze"
DEFAULT_BASE = "https://api.aim.security"


class Settings(TypedDict, total=False):
    api_key: str | None
    api_base: str | None
    inspect_embeddings: bool | None


class ChunkDelta(TypedDict):
    content: NotRequired[str]
    role: NotRequired[str]


class ChunkChoice(TypedDict):
    delta: ChunkDelta


class Chunk(TypedDict):
    choices: list[ChunkChoice]


class CaseInput(TypedDict):
    function: str
    settings: Settings
    request_data: dict[str, Json]
    hook: str
    key_alias: str | None
    answer: dict[str, Json]
    answer_status: int
    output: str
    chunks: list[Chunk]
    frames: list[dict[str, Json]]


class Rejection(BaseModel):
    type: str
    param: str | None
    openai_code: int | str | None


class RecordedCall(TypedDict):
    url: str
    headers: dict[str, str]
    body: Json


class AimInspector(Protocol):
    api_key: str | None
    api_base: str
    ws_api_base: str

    @property
    def inspect_embeddings(self) -> bool: ...

    def get_supported_event_hooks(self) -> list[GuardrailEventHooks]: ...

    async def call_aim_guardrail(
        self, data: dict[str, Json], hook: str, key_alias: str | None
    ) -> dict[str, Json]: ...

    async def call_aim_guardrail_on_output(
        self,
        request_data: dict[str, Json],
        output: str,
        hook: str,
        key_alias: str | None,
    ) -> AimOutputGuardrailResult | None: ...


class AnalysisServer:
    def __init__(self, answer: dict[str, Json], status: int) -> None:
        self.answer = answer
        self.status = status
        self.requests: list[RecordedCall] = []

    async def __call__(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        head = await reader.readuntil(b"\r\n\r\n")
        lines = head.decode("latin-1").split("\r\n")
        request_line = lines[0].split(" ")
        headers: dict[str, str] = {}
        for line in lines[1:]:
            name, separator, value = line.partition(":")
            if separator:
                headers[name.strip().lower()] = value.strip()
        length_text = headers.get("content-length")
        length = 0 if length_text is None else int(length_text)
        payload = await reader.readexactly(length)
        body = TypeAdapter(Json).validate_json(payload)
        self.requests.append(RecordedCall(url=request_line[1], headers=headers, body=body))
        encoded = dumps(self.answer).encode()
        response_head = (
            f"HTTP/1.1 {self.status} OK\r\nContent-Type: application/json\r\n"
            f"Content-Length: {len(encoded)}\r\n\r\n"
        )
        writer.write(response_head.encode() + encoded)
        await writer.drain()
        writer.close()


class FrameServer:
    def __init__(self, frames: Sequence[dict[str, Json]]) -> None:
        self.frames = frames

    async def __call__(self, connection: ServerConnection) -> None:
        for frame in self.frames:
            await connection.send(dumps(frame))


async def streaming_chunks(
    chunks: Sequence[Chunk],
) -> AsyncIterator[ModelResponseStream]:
    for chunk in chunks:
        choices: list[StreamingChoices] = []
        for choice in chunk["choices"]:
            delta = choice["delta"]
            choices.append(
                StreamingChoices(delta=Delta(content=delta.get("content"), role=delta.get("role")))
            )
        yield ModelResponseStream(choices=choices)


def build_guardrail(settings: Settings, api_base: str | None) -> AimInspector:
    base = DEFAULT_BASE if api_base is None else api_base
    return AimGuardrail(
        api_key=settings.get("api_key"),
        api_base=base,
        inspect_embeddings=settings.get("inspect_embeddings"),
        timeout=None,
    )


def request_output(server: AnalysisServer) -> Json:
    if not server.requests:
        raise RuntimeError("the guardrail sent no request")
    call = server.requests[0]
    headers: dict[str, Json] = {
        name: value
        for name, value in call["headers"].items()
        if name.startswith("x-aim-") or name == "authorization"
    }
    view: dict[str, Json] = {
        "path_ok": call["url"] == ANALYZE_PATH,
        "headers": headers,
        "body": call["body"],
    }
    return view


def run_construct(input_value: CaseInput) -> Json:
    settings = input_value["settings"]
    try:
        guardrail = build_guardrail(settings, settings.get("api_base"))
    except AimGuardrailMissingSecrets:
        return {"missing_secrets": True}
    hooks: list[Json] = [hook.value for hook in guardrail.get_supported_event_hooks()]
    view: dict[str, Json] = {
        "api_key": guardrail.api_key,
        "api_base": guardrail.api_base,
        "ws_api_base": guardrail.ws_api_base,
        "inspect_embeddings": guardrail.inspect_embeddings,
        "supported_event_hooks": hooks,
    }
    return view


def rejection(outcome: dict[str, Json] | BaseException) -> Json:
    match outcome:
        case BaseException():
            facts = TypeAdapter(Rejection).validate_python(outcome, from_attributes=True)
            return {
                "status": HTTP_BAD_REQUEST,
                "type": facts.type,
                "param": facts.param,
                "code": facts.openai_code,
            }
        case dict():
            return None


@asynccontextmanager
async def analysis_endpoint(
    input_value: CaseInput,
) -> AsyncGenerator[tuple[AnalysisServer, AimInspector]]:
    server = AnalysisServer(input_value["answer"], input_value["answer_status"])
    with socketserver.TCPServer(("127.0.0.1", 0), socketserver.BaseRequestHandler) as listener:
        http = await asyncio.start_server(server, sock=listener.socket)
        try:
            yield (
                server,
                build_guardrail(
                    input_value["settings"], f"http://127.0.0.1:{listener.server_address[1]}"
                ),
            )
        finally:
            http.close()


async def run_request_async(input_value: CaseInput) -> Json:
    data = TypeAdapter(dict[str, Json]).validate_python(input_value["request_data"])
    async with analysis_endpoint(input_value) as (server, inspector):
        outcome = await asyncio.gather(
            inspector.call_aim_guardrail(
                data, hook=input_value["hook"], key_alias=input_value["key_alias"]
            ),
            return_exceptions=True,
        )
        view: dict[str, Json] = {
            "request": request_output(server),
            "request_data": data,
            "blocked": rejection(outcome[0]),
        }
        return view


async def run_output_async(input_value: CaseInput) -> Json:
    request_data = TypeAdapter(dict[str, Json]).validate_python(input_value["request_data"])
    async with analysis_endpoint(input_value) as (server, inspector):
        result = await inspector.call_aim_guardrail_on_output(
            request_data,
            input_value["output"],
            hook="output",
            key_alias=input_value["key_alias"],
        )
        if result is None:
            raise RuntimeError("the guardrail returned no output verdict")
        output: dict[str, Json] = {
            "detected": result.get("detection_message") is not None,
            "redacted_output": result.get("redacted_output"),
        }
        view: dict[str, Json] = {"request": request_output(server), "output": output}
        return view


async def run_stream_async(input_value: CaseInput) -> Json:
    frames = FrameServer(input_value["frames"])
    user = UserAPIKeyAuth(api_key="aim-oracle-key", key_alias=input_value["key_alias"])
    request_data = TypeAdapter(dict[str, Json]).validate_python(input_value["request_data"])
    contents: list[Json] = []
    blocked = False
    with socketserver.TCPServer(("127.0.0.1", 0), socketserver.BaseRequestHandler) as listener:
        async with serve(frames, sock=listener.socket):
            settings = input_value["settings"]
            guardrail = AimGuardrail(
                api_key=settings.get("api_key"),
                api_base=f"http://127.0.0.1:{listener.server_address[1]}",
                inspect_embeddings=settings.get("inspect_embeddings"),
                timeout=None,
            )
            hook_call = guardrail.async_post_call_streaming_iterator_hook(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/aim/aim.py#L408-L413
                user, streaming_chunks(input_value["chunks"]), request_data
            )
            try:
                async for output in hook_call:
                    content = output.choices[0].delta.content
                    contents.append("" if content is None else content)
            except StreamingCallbackError:
                blocked = True
    view: dict[str, Json] = {"chunks": contents, "blocked": blocked}
    return view


def run(input_value: CaseInput) -> Json:
    function = input_value["function"]
    if function == "construct":
        return run_construct(input_value)
    if function == "request":
        return asyncio.run(run_request_async(input_value))
    if function == "output":
        return asyncio.run(run_output_async(input_value))
    return asyncio.run(run_stream_async(input_value))
