import json
import logging
import os
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Final, Protocol, TypeAlias, runtime_checkable

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.arguments import SessionLike
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.types.llms.openai.core.aliases import FileTypes

LOGGER: Final = logging.getLogger("LiteLLM")
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

FILE_KEY: Final = "$file"
DEFAULT_NAME: Final = "file"
REFUSED: Final[tuple[str, ...]] = ("client", "shared_session")
LOGGER_FN: Final = "logger_fn"
LOGGER_FN_DROPPED: Final = "logger_fn is not called: the endpoint call runs in Rust"


@runtime_checkable
class Readable(Protocol):
    def read(self) -> bytes: ...


@runtime_checkable
class Seekable(Protocol):
    def tell(self) -> int: ...
    def seek(self, position: int) -> int: ...


@runtime_checkable
class Named(Protocol):
    name: str


@runtime_checkable
class PathValue(Protocol):
    def __fspath__(self) -> str | bytes: ...


@runtime_checkable
class TimeoutValue(Protocol):
    connect: float | None
    read: float | None
    write: float | None
    pool: float | None


# everything a family passes in: JSON, a file, a timeout, or an object LiteLLM's own call would take
CallValue: TypeAlias = (
    JsonValue
    | FileTypes
    | bytearray
    | Mapping[str, str]
    | Readable
    | TimeoutValue
    | SessionLike
    | list["CallValue"]
    | dict[str, "CallValue"]
)


@dataclass(frozen=True, slots=True)
class CallPayload:
    text: str
    blobs: list[bytes]


def to_call_payload(values: Mapping[str, CallValue]) -> CallPayload:
    blobs: list[bytes] = []
    encoded: dict[str, JsonValue] = {}
    for name, value in values.items():
        if not is_dropped(name, value):
            encoded[name] = plain_value(name, value, blobs)
    return CallPayload(text=json.dumps(encoded), blobs=blobs)


# LiteLLM's clients and loggers never cross into Rust; proxy headers come from proxy_headers()
def is_dropped(name: str, value: CallValue) -> bool:
    if name in REFUSED and value is not None:
        raise UnsupportedCallArgument(name)
    if name in REFUSED or name == LOGGER_FN:
        if name == LOGGER_FN and value is not None:
            LOGGER.warning(LOGGER_FN_DROPPED)
        return True
    return False


def plain_value(key: str, value: CallValue, blobs: list[bytes]) -> JsonValue:
    match value:
        case bool() | str() | int() | float() | None:
            return JSON.validate_python(value)
        case dict():
            return {name: plain_value(name, item, blobs) for name, item in value.items()}
        case list():
            return [plain_value(key, item, blobs) for item in value]
        case TimeoutValue():
            return JSON.validate_python(timeout_seconds(value))
        case object():
            return file_value(key, value, blobs)


def file_value(key: str, value: CallValue, blobs: list[bytes]) -> JsonValue:
    match value:
        case bytes() as data:
            return file_marker(DEFAULT_NAME, None, data, blobs)
        case bytearray() as data:
            return file_marker(DEFAULT_NAME, None, bytes(data), blobs)
        case tuple():
            return tuple_file(key, value, blobs)
        case PathValue() as path:
            return file_marker(path_name(path), None, content_bytes(key, path), blobs)
        case Readable() as stream:
            data = read_stream(stream)
            return file_marker(stream_name(stream), None, data, blobs)
        case object():
            raise TypeError(f"{key} is neither a JSON value nor a file")


def tuple_file(key: str, items: tuple[CallValue, ...], blobs: list[bytes]) -> JsonValue:
    if len(items) < 2:
        raise TypeError(f"{key} is not a file: a tuple needs a name and its content")
    name = tuple_name(key, items[0])
    content_type = tuple_content_type(key, items[2:3])
    data = content_bytes(key, items[1])
    return file_marker(name, content_type, data, blobs)


def tuple_name(key: str, value: CallValue) -> str:
    match value:
        case str() as given:
            return given or DEFAULT_NAME
        case None:
            return DEFAULT_NAME
        case object():
            raise TypeError(f"{key} is not a file: the tuple name is not text")


def tuple_content_type(key: str, extra: tuple[CallValue, ...]) -> str | None:
    if not extra:
        return None
    match extra[0]:
        case str() as kind:
            return kind
        case None:
            return None
        case object():
            raise TypeError(f"{key} is not a file: the tuple content type is not text")


def content_bytes(key: str, value: CallValue) -> bytes:
    match value:
        case bytes() as data:
            return data
        case bytearray() as data:
            return bytes(data)
        case PathValue() as path:
            return Path(os.fsdecode(path.__fspath__())).read_bytes()
        case Readable() as stream:
            return read_stream(stream)
        case object():
            raise TypeError(f"{key} is not a file: the tuple content is not bytes")


def path_name(path: PathValue) -> str:
    file_path = Path(os.fsdecode(path.__fspath__()))
    return file_path.name or DEFAULT_NAME


def stream_name(stream: Readable) -> str:
    match stream:
        case Named() as named:
            name = Path(named.name).name
            if name:
                return name
        case object():
            pass
    return DEFAULT_NAME


def read_stream(stream: Readable) -> bytes:
    match stream:
        case Seekable():
            # the caller reads the same file object again, so it is read from the start and put back
            position = stream.tell()
            stream.seek(0)
            try:
                return stream.read()
            finally:
                stream.seek(position)
        case object():
            return stream.read()


def timeout_seconds(timeout: TimeoutValue) -> float | None:
    phases = [
        phase
        for phase in (timeout.connect, timeout.read, timeout.write, timeout.pool)
        if phase is not None
    ]
    if not phases:
        return None
    return max(phases)


def file_marker(
    name: str,
    content_type: str | None,
    data: bytes,
    blobs: list[bytes],
) -> JsonValue:
    index = len(blobs)
    blobs.append(data)
    return {FILE_KEY: {"name": name, "blob": index, "content_type": content_type}}
