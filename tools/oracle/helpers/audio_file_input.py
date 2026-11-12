import base64
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import BinaryIO, Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.litellm_core_utils.audio_utils.utils import (
    calculate_request_duration,
    get_audio_file_content_hash,
    get_audio_file_for_health_check,
    get_audio_file_name,
    normalize_transcription_language_to_bcp47,
    process_audio_file,
)
from litellm.types.utils import FileTypes

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
ReadMode: TypeAlias = Literal["content", "fail", "missing"]
SeekMode: TypeAlias = Literal["works", "unsupported", "fails"]


class BytesFile(TypedDict):
    kind: Literal["bytes"]
    content: str


class ByteArrayFile(TypedDict):
    kind: Literal["bytearray"]
    content: str


class TextFile(TypedDict):
    kind: Literal["text"]
    content: str


class PathFile(TypedDict):
    kind: Literal["path"]
    path: str


class UnsupportedFile(TypedDict):
    kind: Literal["unsupported"]
    class_name: str


class StreamFile(TypedDict):
    kind: Literal["stream"]
    name: str
    content: NotRequired[str]
    position: NotRequired[int]
    read: NotRequired[ReadMode]
    seek: NotRequired[SeekMode]


class TupleFile(TypedDict):
    kind: Literal["tuple"]
    filename: str | None
    content: NotRequired["ContentSpec"]
    mime: NotRequired[str]


ContentSpec: TypeAlias = (
    BytesFile | ByteArrayFile | TextFile | PathFile | StreamFile | UnsupportedFile
)
FileSpec: TypeAlias = ContentSpec | TupleFile


class ProcessCall(TypedDict):
    function: Literal["process"]
    file: FileSpec
    files: NotRequired[dict[str, str]]


class HashCall(TypedDict):
    function: Literal["hash"]
    file: FileSpec
    files: NotRequired[dict[str, str]]


class NameCall(TypedDict):
    function: Literal["name"]
    file: FileSpec


class LanguageCall(TypedDict):
    function: Literal["language"]
    language: str


class DurationCall(TypedDict):
    function: Literal["duration"]
    file: FileSpec
    files: NotRequired[dict[str, str]]


class HealthCall(TypedDict):
    function: Literal["health"]


Call: TypeAlias = ProcessCall | HashCall | NameCall | LanguageCall | DurationCall | HealthCall


class Input(TypedDict):
    calls: list[Call]


class Reader:
    def __init__(self, name: str, content: bytes, position: int) -> None:
        self.name = name
        self.content = content
        self.position = position
        self.read_mode: ReadMode = "content"

    def read(self) -> bytes | None:
        if self.read_mode == "fail":
            raise OSError("the fixture reader fails")
        if self.read_mode == "missing":
            return None
        start = min(self.position, len(self.content))
        self.position = len(self.content)
        return self.content[start:]


class SeekableReader(Reader):
    def __init__(
        self, name: str, content: bytes, position: int, *, fail_seek: bool
    ) -> None:
        super().__init__(name, content, position)
        self.fail_seek = fail_seek

    def tell(self) -> int:
        return self.position

    def seek(self, position: int) -> None:
        if self.fail_seek:
            raise OSError("the fixture seek fails")
        self.position = position


class FixedReader(Reader):
    def tell(self) -> int:
        return self.position


def run(input_value: Input) -> Json:
    with TemporaryDirectory() as directory:
        root = Path(directory)
        return [_run_one(call, root) for call in input_value["calls"]]


def _run_one(call: Call, directory: Path) -> Json:
    outcome: Json
    try:
        outcome = {"ok": True, "value": _evaluate(call, directory)}
    except (OSError, ValueError):
        outcome = {"ok": False}
    return outcome


def _evaluate(call: Call, directory: Path) -> Json:
    match call["function"]:
        case "process":
            return _process(call, directory)
        case "hash":
            return _hash(call, directory)
        case "name":
            return _name(call, directory)
        case "language":
            return normalize_transcription_language_to_bcp47(call["language"])
        case "duration":
            return _duration(call, directory)
        case "health":
            return _health()


def _process(call: ProcessCall, directory: Path) -> Json:
    file_obj, _reader = _fixture(call["file"], directory, _files(call))
    processed = process_audio_file(file_obj)
    return {
        "filename": processed.filename,
        "content_type": processed.content_type,
        "content": base64.b64encode(processed.file_content).decode(),
    }


def _hash(call: HashCall, directory: Path) -> Json:
    file_obj, reader = _fixture(call["file"], directory, _files(call))
    digest = get_audio_file_content_hash(file_obj=file_obj)
    return {"hash": digest, "position": _position(reader)}


def _name(call: NameCall, directory: Path) -> Json:
    file_obj, _reader = _fixture(call["file"], directory, {})
    return get_audio_file_name(file_obj=file_obj)


def _duration(call: DurationCall, directory: Path) -> Json:
    file_obj, reader = _fixture(call["file"], directory, _files(call))
    seconds = calculate_request_duration(file=file_obj)
    return {"duration": seconds, "position": _position(reader)}


def _health() -> Json:
    handle = cast(BinaryIO, get_audio_file_for_health_check())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L255
    content = handle.read()
    handle.close()
    return {"content": base64.b64encode(content).decode()}


def _position(reader: Reader | None) -> int | None:
    if reader is None:
        return None
    return reader.position


def _files(call: ProcessCall | HashCall | DurationCall) -> dict[str, str]:
    files = call.get("files")
    if files is None:
        return {}
    return files


def _fixture(
    spec: FileSpec, directory: Path, files: dict[str, str]
) -> tuple[FileTypes, Reader | None]:
    value: FileTypes
    reader: Reader | None = None
    match spec["kind"]:
        case "bytes":
            value = base64.b64decode(spec["content"])
        case "bytearray":
            value = cast(FileTypes, bytearray(base64.b64decode(spec["content"])))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L58
        case "text":
            value = cast(FileTypes, spec["content"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L62
        case "path":
            value = _path(spec["path"], directory, files)
        case "stream":
            stream_reader = _reader(spec)
            value = cast(FileTypes, stream_reader)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L105
            reader = stream_reader
        case "tuple":
            content_spec = spec.get("content")
            if content_spec is None:
                value = cast(FileTypes, (spec["filename"],))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L81
            else:
                inner, inner_reader = _fixture(content_spec, directory, files)
                mime = spec.get("mime")
                if mime is None:
                    value = cast(FileTypes, (spec["filename"], inner))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L80
                else:
                    value = cast(FileTypes, (spec["filename"], inner, mime))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L80
                reader = inner_reader
        case "unsupported":
            value = cast(FileTypes, 0)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/audio_utils/utils.py#L113
    return value, reader


def _path(value: str, directory: Path, files: dict[str, str]) -> Path:
    content = files.get(value)
    if content is None:
        return Path(value)
    name = value.rsplit("/", maxsplit=1)[-1]
    destination = directory / name
    destination.write_bytes(base64.b64decode(content))
    return destination


def _reader(spec: StreamFile) -> Reader:
    content = spec.get("content")
    raw = b"" if content is None else base64.b64decode(content)
    position = spec.get("position")
    reader = _new_reader(
        spec["name"], raw, 0 if position is None else position, spec.get("seek")
    )
    mode = spec.get("read")
    if mode is not None:
        reader.read_mode = mode
    return reader


def _new_reader(
    name: str, content: bytes, position: int, seek: SeekMode | None
) -> SeekableReader | FixedReader:
    if seek is None or seek == "works":
        return SeekableReader(name=name, content=content, position=position, fail_seek=False)
    if seek == "fails":
        return SeekableReader(name=name, content=content, position=position, fail_seek=True)
    return FixedReader(name=name, content=content, position=position)
