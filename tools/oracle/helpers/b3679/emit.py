import asyncio
from collections.abc import Sequence
from typing import Final, Literal, TypeAlias, TypedDict, TypeVar

from fastapi.exceptions import HTTPException
from litellm.llms.openai.chat.guardrail_translation.handler import (
    OpenAIChatCompletionsHandler,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from starlette.exceptions import HTTPException as RaisedError
from typing_extensions import override

from .chunks import HOOKS, JSON, error_tag

A2A_CALL_TYPE: Final = "send_message"
Element = TypeVar("Element")


class TranslationScript(TypedDict):
    error_items: list[str] | None


class Emit(TypedDict):
    call_type: str | None
    responses_so_far: list[Json]
    request_data: dict[str, Json]
    stream_started: bool
    responses_yielded: list[Json] | None
    translation: TranslationScript | None


class EmissionCall(Emit):
    function: Literal["http_error_emission"]
    status_code: int
    detail: Json


class A2aRequestIdCall(TypedDict):
    function: Literal["a2a_request_id"]
    responses_so_far: list[Json]
    request_data: dict[str, Json]


class A2aErrorCall(TypedDict):
    function: Literal["a2a_error_chunk"]
    status_code: int
    detail: Json
    request_id: Json


class A2aError(TypedDict):
    code: int
    message: Json
    data: dict[str, Json]


class A2aChunk(TypedDict):
    jsonrpc: str
    id: Json
    error: A2aError


Frame: TypeAlias = str | A2aChunk
FRAME: Final[TypeAdapter[Frame]] = TypeAdapter(Frame)
A2A_CHUNK: Final[TypeAdapter[A2aChunk]] = TypeAdapter(A2aChunk)


class ScriptedTranslation(OpenAIChatCompletionsHandler):
    def __init__(self, items: list[str] | None) -> None:
        super().__init__()
        self.items = items

    @override
    def build_stream_error_items(
        self,
        exc: HTTPException,
        responses_so_far: Sequence[Element] | None = None,
    ) -> Sequence[bytes] | None:
        if self.items is None:
            return None
        count = 0 if responses_so_far is None else len(responses_so_far)
        return [*(item.encode() for item in self.items), f"items={count}".encode()]


async def collect(error: HTTPException, request: Emit) -> list[Frame]:
    script = request["translation"]
    translation = None if script is None else ScriptedTranslation(script["error_items"])
    return [
        FRAME.validate_python(item)
        async for item in HOOKS.emit_streaming_http_error(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L438-L447
            error,
            request["call_type"],
            request["responses_so_far"],
            request["request_data"],
            translation,
            request["stream_started"],
            request["responses_yielded"],
        )
    ]


def emission(call: EmissionCall) -> Json:
    error = HTTPException(status_code=call["status_code"], detail=call["detail"])
    try:
        frames = asyncio.run(collect(error, call))
    except RaisedError as raised:
        return error_tag(raised)
    items: list[Json] = [JSON.validate_python(frame) for frame in frames]
    return {"items": items}


def a2a_frame(
    error: HTTPException, responses_so_far: list[Json], request_data: dict[str, Json]
) -> Frame:
    request = Emit(
        call_type=A2A_CALL_TYPE,
        responses_so_far=responses_so_far,
        request_data=request_data,
        stream_started=False,
        responses_yielded=None,
        translation=None,
    )
    return asyncio.run(collect(error, request))[0]


def request_id(call: A2aRequestIdCall) -> Json:
    error = HTTPException(status_code=400, detail="denied")
    frame = a2a_frame(error, call["responses_so_far"], call["request_data"])
    return A2A_CHUNK.validate_python(frame)["id"]


def error_chunk(call: A2aErrorCall) -> Json:
    error = HTTPException(status_code=call["status_code"], detail=call["detail"])
    request_data: dict[str, Json] = {"body": {"id": call["request_id"]}}
    return JSON.validate_python(a2a_frame(error, [], request_data))
