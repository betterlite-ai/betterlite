import base64
import io
from collections.abc import Mapping, Sequence
from typing import IO, Annotated, Literal, Protocol, TypeAlias, TypedDict

from pydantic import Field, JsonValue
from typing_extensions import override

Kind = Literal["bytes", "text", "stream"]
Failure = Literal["none", "tell", "seek", "read", "restore"]
Content: TypeAlias = IO[bytes] | bytes | str
StoredFiles: TypeAlias = (
    Content
    | tuple[str | None, Content]
    | tuple[str | None, Content, str | None]
    | tuple[str | None, Content, str | None, Mapping[str, str]]
)
Parts: TypeAlias = Mapping[str, StoredFiles] | Sequence[tuple[str, StoredFiles]]
Part: TypeAlias = tuple[str, tuple[str | None, Content, str | None]]


class BytesFixture(TypedDict):
    kind: Literal["bytes"]
    data_base64: str


class TextFixture(TypedDict):
    kind: Literal["text"]
    value: str


class BytesIoFixture(TypedDict):
    kind: Literal["bytes_io"]
    data_base64: str
    position: int


class BufferedReaderFixture(TypedDict):
    kind: Literal["buffered_reader"]
    data_base64: str
    position: int
    name: str


class FailingFixture(TypedDict):
    kind: Literal["failing_reader"]
    data_base64: str
    position: int
    failure: Failure


Fixture: TypeAlias = Annotated[
    BytesFixture | TextFixture | BytesIoFixture | BufferedReaderFixture | FailingFixture,
    Field(discriminator="kind"),
]


class CursorStream(Protocol):
    def cursor(self) -> int: ...


class NamedBytesIO(io.BytesIO):
    def __init__(self, data: bytes, name: str) -> None:
        super().__init__(data)
        self.name = name

    def cursor(self) -> int:
        return self.tell()


class FaultyBytesIO(io.BytesIO):
    def __init__(self, data: bytes, position: int, failure: Failure) -> None:
        super().__init__(data)
        self.failure = failure
        self.seeks = 0
        self.position = position
        io.BytesIO.seek(self, position)

    @override
    def tell(self) -> int:
        if self.failure == "tell":
            raise OSError("tell failed")
        return super().tell()

    @override
    def seek(self, offset: int, whence: int = 0) -> int:
        self.seeks += 1
        if self.failure == "seek" and self.seeks == 1:
            raise OSError("seek failed")
        if self.failure == "restore" and self.seeks > 1:
            raise OSError("restore failed")
        self.position = super().seek(offset, whence)
        return self.position

    @override
    def read(self, size: int | None = -1) -> bytes:
        if self.failure == "read":
            raise OSError("read failed")
        chunk = super().read(size)
        self.position = super().tell()
        return chunk

    def cursor(self) -> int:
        return self.position


class FixtureState:
    def __init__(
        self,
        content: Content,
        kind: Kind,
        body: JsonValue,
        stream: CursorStream | None,
    ) -> None:
        self.content = content
        self.kind = kind
        self.body = body
        self.stream = stream

    def cursor(self) -> int | None:
        if self.stream is None:
            return None
        return self.stream.cursor()

    def rendered(self, part: Part) -> JsonValue:
        field, (filename, content, content_type) = part
        rendered: dict[str, JsonValue] = {
            "field": field,
            "filename": filename,
            "content_type": content_type,
            "body_kind": self.kind,
            "body": self.body,
            "original": content is self.content,
        }
        return rendered

    def result(self, parts: list[Part]) -> JsonValue:
        rendered: list[JsonValue] = [self.rendered(part) for part in parts]
        return {"parts": rendered, "position": self.cursor()}


def fixtures(spec: Fixture) -> FixtureState:
    match spec["kind"]:
        case "bytes":
            data = base64.b64decode(spec["data_base64"])
            return FixtureState(data, "bytes", base64.b64encode(data).decode("ascii"), None)
        case "text":
            return FixtureState(spec["value"], "text", spec["value"], None)
        case "bytes_io":
            data = base64.b64decode(spec["data_base64"])
            stream = FaultyBytesIO(data, spec["position"], "none")
            return FixtureState(stream, "stream", None, stream)
        case "buffered_reader":
            data = base64.b64decode(spec["data_base64"])
            raw = NamedBytesIO(data, spec["name"])
            buffered = io.BufferedReader(raw)
            buffered.seek(spec["position"])
            return FixtureState(buffered, "stream", None, raw)
        case "failing_reader":
            data = base64.b64decode(spec["data_base64"])
            faulty = FaultyBytesIO(data, spec["position"], spec["failure"])
            return FixtureState(faulty, "stream", None, faulty)
