import asyncio
import base64
import time
from collections.abc import AsyncGenerator, AsyncIterator, Mapping
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, NamedTuple, Protocol, TypedDict, runtime_checkable

import httpx
import litellm
from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.files.transformation import VertexAIFilesConfig
from litellm.types.llms.openai import HttpxBinaryResponseContent
from pydantic import ConfigDict, JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class FixedUuid:
    def __init__(self) -> None:
        self.issued = 0

    def uuid4(self) -> UUID:
        issued = self.issued
        self.issued += 1
        return UUID(int=issued)


class StreamedFile(NamedTuple):
    stream_iterator: AsyncIterator[bytes]
    headers: Mapping[str, str]


@runtime_checkable
class FilesConfig(Protocol):
    def transform_file_content_response(  # ast-grep-ignore: no-long-names
        self,
        raw_response: httpx.Response,
        logging_obj: Logging,
        litellm_params: Mapping[str, JsonValue],
    ) -> HttpxBinaryResponseContent: ...

    async def transform_file_content_stream(
        self,
        *,
        stream_iterator: AsyncGenerator[bytes, None],
        headers: Mapping[str, str],
        request_url: str,
        logging_obj: Logging,
        litellm_params: Mapping[str, JsonValue],
    ) -> StreamedFile: ...


# LiteLLM's __init__ is untyped, and both download methods read no instance state
CONFIG: Final[FilesConfig] = TypeAdapter(FilesConfig, config=ARBITRARY).validate_python(
    VertexAIFilesConfig.__new__(VertexAIFilesConfig)
)


class BufferInput(TypedDict):
    operation: Literal["buffer"]
    content: str
    url: str
    headers: dict[str, str]
    disabled: bool
    now: int


class StreamInput(TypedDict):
    operation: Literal["stream"]
    chunks: list[str]
    url: str
    headers: dict[str, str]
    disabled: bool
    now: int


Input = BufferInput | StreamInput


async def _chunks(parts: list[bytes]) -> AsyncGenerator[bytes, None]:
    for part in parts:
        yield part


def _logging(now: int) -> Logging:
    return Logging(
        model="",
        messages=None,
        stream=False,
        call_type="",
        start_time=datetime.fromtimestamp(now, tz=UTC),
        litellm_call_id="",
        function_id="",
    )


def _response(content: bytes, url: str, headers: Mapping[str, str]) -> httpx.Response:
    return httpx.Response(
        status_code=200,
        content=content,
        headers=headers,
        request=httpx.Request(method="GET", url=url),
    )


def _rows(original: bytes, content: bytes) -> JsonValue:
    if content == original:
        return None
    return [JSON.validate_json(line) for line in content.decode().split("\n") if line.strip()]


def _result(original: bytes, content: bytes, headers: Mapping[str, str]) -> JsonValue:
    lowered = {key.lower(): value for key, value in headers.items()}
    others: dict[str, JsonValue] = {
        key: value for key, value in lowered.items() if key != "content-length"
    }
    return {
        "content": base64.b64encode(content).decode(),
        "content_length": lowered.get("content-length"),
        "headers": others,
        "rows": _rows(original, content),
    }


def _buffer_result(input_value: BufferInput) -> JsonValue:
    content = base64.b64decode(input_value["content"])
    response = _response(content, input_value["url"], input_value["headers"])
    logging_obj = _logging(input_value["now"])
    litellm.disable_vertex_batch_output_transformation = input_value["disabled"]
    litellm_uuid.uuid4 = FixedUuid().uuid4
    wall_clock = time.time
    time.time = partial(float, input_value["now"])
    try:
        transformed = CONFIG.transform_file_content_response(
            raw_response=response,
            logging_obj=logging_obj,
            litellm_params={},
        )
    finally:
        time.time = wall_clock
    return _result(content, transformed.response.content, transformed.response.headers)


async def _stream_result(input_value: StreamInput) -> JsonValue:
    parts = [base64.b64decode(chunk) for chunk in input_value["chunks"]]
    logging_obj = _logging(input_value["now"])
    litellm.disable_vertex_batch_output_transformation = input_value["disabled"]
    litellm_uuid.uuid4 = FixedUuid().uuid4
    wall_clock = time.time
    time.time = partial(float, input_value["now"])
    try:
        transformed = await CONFIG.transform_file_content_stream(
            stream_iterator=_chunks(parts),
            headers=input_value["headers"],
            request_url=input_value["url"],
            logging_obj=logging_obj,
            litellm_params={},
        )
        gathered = bytearray()
        async for chunk in transformed.stream_iterator:
            gathered.extend(chunk)
    finally:
        time.time = wall_clock
    return _result(b"".join(parts), bytes(gathered), transformed.headers)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "buffer":
        return _buffer_result(input_value)
    return asyncio.run(_stream_result(input_value))
