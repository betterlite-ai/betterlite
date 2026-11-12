import base64
import io
from typing import Final, Literal, Protocol, TypedDict, cast

import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.images import utils as image_utils
from litellm.images.utils import ImageEditRequestUtils
from litellm.llms.base_llm.image_edit.transformation import BaseImageEditConfig
from litellm.types.images.main import ImageEditOptionalRequestParams
from pydantic import JsonValue
from typing_extensions import override

FixtureKind = Literal[
    "bytes",
    "bytes_io",
    "buffered_reader",
    "seekable",
    "nonseekable",
    "no_read",
    "fail_tell",
    "fail_seek",
    "fail_read",
    "fail_restore",
    "non_bytes_read",
]
Failure = Literal["none", "tell", "seek", "read", "restore", "non_bytes"]

_FAILURES: Final[dict[FixtureKind, Failure]] = {
    "bytes": "none",
    "bytes_io": "none",
    "buffered_reader": "none",
    "seekable": "none",
    "nonseekable": "none",
    "no_read": "none",
    "fail_tell": "tell",
    "fail_seek": "seek",
    "fail_read": "read",
    "fail_restore": "restore",
    "non_bytes_read": "non_bytes",
}


class SelectCall(TypedDict):
    function: Literal["select"]
    supported: list[str]
    params: dict[str, JsonValue]


class OptionalCall(TypedDict):
    function: Literal["optional"]
    model: str
    supported: list[str]
    global_drop_params: bool
    call_drop_params: bool | None
    additional_drop_params: list[str] | None
    params: dict[str, JsonValue]


class Fixture(TypedDict):
    kind: FixtureKind
    data_base64: str
    position: int
    name: str | None


class ContentCall(TypedDict):
    function: Literal["content"]
    fixture: Fixture


class DetectedCall(TypedDict):
    function: Literal["detected"]
    detection: str | None
    raises: bool


Call = SelectCall | OptionalCall | ContentCall | DetectedCall


class Input(TypedDict):
    calls: list[Call]


class SeekableReader:
    def __init__(self, data: bytes, position: int, failure: Failure, name: str | None) -> None:
        self.data = data
        self.position = position
        self.failure = failure
        self.name = name
        self.seeks = 0

    def tell(self) -> int:
        if self.failure == "tell":
            raise OSError("tell failed")
        return self.position

    def seek(self, offset: int, whence: int = 0) -> int:
        if whence != 0:
            raise OSError("unsupported whence")
        self.seeks += 1
        if self.failure == "seek" and self.seeks == 1:
            raise OSError("seek failed")
        if self.failure == "restore" and self.seeks > 1:
            raise OSError("restore failed")
        self.position = offset
        return offset

    def read(self, size: int = -1) -> bytes | None:
        if self.failure == "read":
            raise OSError("read failed")
        if self.failure == "non_bytes":
            return None
        end = self.position + size
        chunk = self.data[self.position : end]
        self.position += len(chunk)
        return chunk


class StreamReader:
    def __init__(self, data: bytes, position: int, failure: Failure) -> None:
        self.data = data
        self.position = position
        self.failure = failure

    def read(self, size: int = -1) -> bytes:
        if self.failure == "read":
            raise OSError("read failed")
        end = self.position + size
        chunk = self.data[self.position : end]
        self.position += len(chunk)
        return chunk


class Opaque:
    pass


class Detector:
    def __init__(self, detection: str | None, *, raises: bool) -> None:
        self.detection = detection
        self.raises = raises

    def __call__(self, _image_data: bytes) -> str | None:
        if self.raises:
            raise ValueError("detection failed")
        return self.detection


class Provider(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        image_edit_optional_params: ImageEditOptionalRequestParams,
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


class RecordingProvider(Provider):
    def __init__(self, supported: list[str]) -> None:
        self.supported = supported

    @override
    def get_supported_openai_params(self, model: str) -> list[str]:
        return self.supported

    @override
    def map_openai_params(
        self,
        image_edit_optional_params: ImageEditOptionalRequestParams,
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]:
        return {
            "model": model,
            "drop_params": drop_params,
            "params": cast("dict[str, JsonValue]", dict(image_edit_optional_params)),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/images/main.py#L8-L35
        }


def select_call(call: SelectCall) -> JsonValue:
    selected = ImageEditRequestUtils.get_requested_image_edit_optional_param(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/utils.py#L15-L69
        params=call["params"],
        provider_supported_params=call["supported"],
    )
    return cast("dict[str, JsonValue]", dict(selected))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/images/main.py#L8-L35


def optional_call(call: OptionalCall) -> JsonValue:
    provider = cast(BaseImageEditConfig, RecordingProvider(call["supported"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py#L33
    litellm.drop_params = call["global_drop_params"]
    try:
        mapped = ImageEditRequestUtils.get_optional_params_image_edit(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/utils.py#L15-L69
            model=call["model"],
            image_edit_provider_config=provider,
            image_edit_optional_params=cast(ImageEditOptionalRequestParams, call["params"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/images/main.py#L8-L35
            drop_params=call["call_drop_params"],
            additional_drop_params=call["additional_drop_params"],
        )
    except UnsupportedParamsError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "value": cast("dict[str, JsonValue]", mapped)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/utils.py#L15-L69


def content_call(call: ContentCall) -> JsonValue:
    fixture = call["fixture"]
    data = base64.b64decode(fixture["data_base64"])
    kind = fixture["kind"]
    name = fixture["name"]
    match kind:
        case "bytes":
            return {"mime": ImageEditRequestUtils.get_image_content_type(data), "name": name}
        case "no_read":
            return {"mime": ImageEditRequestUtils.get_image_content_type(Opaque()), "name": name}
        case "bytes_io":
            stream = io.BytesIO(data)
            stream.seek(fixture["position"])
            mime = ImageEditRequestUtils.get_image_content_type(stream)
            return {"mime": mime, "name": name, "position": stream.tell()}
        case "buffered_reader":
            buffered = io.BufferedReader(io.BytesIO(data))
            buffered.seek(fixture["position"])
            mime = ImageEditRequestUtils.get_image_content_type(buffered)
            return {"mime": mime, "name": name, "position": buffered.tell()}
        case "nonseekable":
            stream_reader = StreamReader(data, fixture["position"], _FAILURES[kind])
            mime = ImageEditRequestUtils.get_image_content_type(stream_reader)
            return {"mime": mime, "name": name, "position": stream_reader.position}
        case (
            "seekable"
            | "fail_tell"
            | "fail_seek"
            | "fail_read"
            | "fail_restore"
            | "non_bytes_read"
        ):
            reader = SeekableReader(data, fixture["position"], _FAILURES[kind], name)
            mime = ImageEditRequestUtils.get_image_content_type(reader)
            return {"mime": mime, "name": name, "position": reader.position}


def patched_content_type(detector: Detector) -> str:
    original = image_utils.get_image_type  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/utils.py#L8
    image_utils.get_image_type = detector  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/utils.py#L8
    try:
        return ImageEditRequestUtils.get_image_content_type(b"")
    finally:
        image_utils.get_image_type = original  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/images/utils.py#L8


def detected_call(call: DetectedCall) -> JsonValue:
    detector = Detector(call["detection"], raises=call["raises"])
    return {"mime": patched_content_type(detector)}


def run_call(call: Call) -> JsonValue:
    match call["function"]:
        case "select":
            return select_call(call)
        case "optional":
            return optional_call(call)
        case "content":
            return content_call(call)
        case "detected":
            return detected_call(call)


def run(input_value: Input) -> JsonValue:
    return [
        run_call(call)
        for call in input_value["calls"]
    ]
