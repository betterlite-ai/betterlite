import asyncio
from collections.abc import AsyncIterator, Sequence
from json import dumps, loads
from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.aim.aim import (
    AimGuardrail,
    AimGuardrailMissingSecrets,
    AimOutputGuardrailResult,
)
from litellm.proxy.proxy_server import StreamingCallbackError
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from websockets.asyncio.server import ServerConnection, serve

HTTP_BAD_REQUEST = 400


class Settings(TypedDict, total=False):
    api_key: str | None
    api_base: str | None
    inspect_embeddings: bool | None
    guardrail_name: str | None


class ConstructInput(TypedDict):
    function: Literal["construct"]
    settings: Settings
    environment: dict[str, str]


class RequestInput(TypedDict):
    function: Literal["request"]
    settings: Settings
    environment: dict[str, str]
    request_data: dict[str, Json]
    hook: Literal["pre_call", "moderation"]
    key_alias: str | None
    answer: dict[str, Json]
    answer_status: int


class OutputInput(TypedDict):
    function: Literal["output"]
    settings: Settings
    environment: dict[str, str]
    request_data: dict[str, Json]
    output: str
    key_alias: str | None
    answer: dict[str, Json]
    answer_status: int


class StreamInput(TypedDict):
    function: Literal["stream"]
    settings: Settings
    environment: dict[str, str]
    request_data: dict[str, Json]
    key_alias: str | None
    chunks: list[dict[str, Json]]
    frames: list[dict[str, Json]]


Input: TypeAlias = ConstructInput | RequestInput | OutputInput | StreamInput


class RecordedCall(TypedDict):
    url: str
    headers: dict[str, str]
    body: Json


class AimInspector(Protocol):
    api_key: str | None
    api_base: str
    ws_api_base: str
    inspect_embeddings: bool
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

    async def __call__(
        self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter
    ) -> None:
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
        body = TypeAdapter(Json).validate_python(loads(payload.decode()))
        self.requests.append(
            RecordedCall(url=request_line[1], headers=headers, body=body)
        )
        encoded = dumps(self.answer).encode()
        head = (
            f"HTTP/1.1 {self.status} OK\r\nContent-Type: application/json\r\n"
            f"Content-Length: {len(encoded)}\r\n\r\n"
        )
        writer.write(head.encode() + encoded)
        await writer.drain()
        writer.close()


class FrameServer:
    def __init__(self, frames: Sequence[dict[str, Json]]) -> None:
        self.frames = frames

    async def __call__(self, connection: ServerConnection) -> None:
        for frame in self.frames:
            await connection.send(dumps(frame))


async def streaming_chunks(
    chunks: Sequence[dict[str, Json]]
) -> AsyncIterator[ModelResponseStream]:
    for chunk in chunks:
        yield ModelResponseStream.model_validate(chunk)


def listening_port(server: asyncio.Server) -> int:
    sockets = server.sockets
    if not sockets:
        raise RuntimeError("the analysis server is not listening")
    return TypeAdapter(int).validate_python(sockets[0].getsockname()[1])


def build_guardrail(settings: Settings, api_base: str | None) -> AimInspector:
    base = "https://api.aim.security" if api_base is None else api_base
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
    headers = {
        name: value
        for name, value in call["headers"].items()
        if name.startswith("x-aim-") or name == "authorization"
    }
    return TypeAdapter(Json).validate_python(
        {"path": call["url"], "headers": headers, "body": call["body"]}
    )


def rejection(error: ProxyException) -> Json:
    return {
        "status": HTTP_BAD_REQUEST,
        "type": error.type,
        "param": error.param,
        "code": error.openai_code,
    }


def run_construct(input_value: ConstructInput) -> Json:
    try:
        settings = input_value["settings"]
        guardrail = build_guardrail(settings, settings.get("api_base"))
    except AimGuardrailMissingSecrets:
        return {"missing_secrets": True}
    hooks = [hook.value for hook in AimGuardrail.get_supported_event_hooks()]
    return TypeAdapter(Json).validate_python(
        {
            "api_key": guardrail.api_key,
            "api_base": guardrail.api_base,
            "ws_api_base": guardrail.ws_api_base,
            "inspect_embeddings": guardrail.inspect_embeddings,
            "supported_event_hooks": hooks,
        }
    )


async def run_request_async(input_value: RequestInput) -> Json:
    server = AnalysisServer(input_value["answer"], input_value["answer_status"])
    http = await asyncio.start_server(server, "127.0.0.1", 0)
    try:
        inspector = build_guardrail(
            input_value["settings"], f"http://127.0.0.1:{listening_port(http)}"
        )
        data = TypeAdapter(dict[str, Json]).validate_python(input_value["request_data"])
        blocked: Json = None
        try:
            await inspector.call_aim_guardrail(
                data,
                hook=input_value["hook"],
                key_alias=input_value["key_alias"],
            )
        except ProxyException as error:
            blocked = rejection(error)
    finally:
        http.close()
    return TypeAdapter(Json).validate_python(
        {
            "request": request_output(server),
            "request_data": data,
            "blocked": blocked,
        }
    )


async def run_output_async(input_value: OutputInput) -> Json:
    server = AnalysisServer(input_value["answer"], input_value["answer_status"])
    http = await asyncio.start_server(server, "127.0.0.1", 0)
    try:
        inspector = build_guardrail(
            input_value["settings"], f"http://127.0.0.1:{listening_port(http)}"
        )
        request_data = TypeAdapter(dict[str, Json]).validate_python(
            input_value["request_data"]
        )
        result = await inspector.call_aim_guardrail_on_output(
            request_data,
            input_value["output"],
            hook="output",
            key_alias=input_value["key_alias"],
        )
    finally:
        http.close()
    if result is None:
        raise RuntimeError("the guardrail returned no output verdict")
    message = result.get("detection_message")
    return TypeAdapter(Json).validate_python(
        {
            "request": request_output(server),
            "output": {
                "detected": message is not None,
                "redacted_output": result.get("redacted_output"),
            },
        }
    )


async def run_stream_async(input_value: StreamInput) -> Json:
    frames = FrameServer(input_value["frames"])
    user = UserAPIKeyAuth(api_key="aim-oracle-key", key_alias=input_value["key_alias"])
    request_data = TypeAdapter(dict[str, Json]).validate_python(
        input_value["request_data"]
    )
    contents: list[str] = []
    blocked = False
    async with serve(frames, "127.0.0.1", 0) as socket:
        port = TypeAdapter(int).validate_python(
            next(iter(socket.sockets)).getsockname()[1]
        )
        settings = input_value["settings"]
        guardrail = AimGuardrail(
            api_key=settings.get("api_key"),
            api_base=f"http://127.0.0.1:{port}",
            inspect_embeddings=settings.get("inspect_embeddings"),
            timeout=None,
        )
        chunks = streaming_chunks(input_value["chunks"])
        hook_call = guardrail.async_post_call_streaming_iterator_hook(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/aim/aim.py#L408-L447
            user, chunks, request_data
        )
        try:
            async for output in hook_call:
                content = output.choices[0].delta.content
                contents.append("" if content is None else content)
        except StreamingCallbackError:
            blocked = True
    return TypeAdapter(Json).validate_python({"chunks": contents, "blocked": blocked})


def run(input_value: Input) -> Json:
    if input_value["function"] == "construct":
        return run_construct(input_value)
    if input_value["function"] == "request":
        return asyncio.run(run_request_async(input_value))
    if input_value["function"] == "output":
        return asyncio.run(run_output_async(input_value))
    return asyncio.run(run_stream_async(input_value))
