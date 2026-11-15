import base64
import io
from collections.abc import Callable
from typing import Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
from litellm.llms.black_forest_labs.image_edit.transformation import (
    BlackForestLabsImageEditConfig,
)
from litellm.types.utils import ImageResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)
MODEL = "black_forest_labs/flux-kontext-pro"


class BytesSpec(TypedDict):
    kind: Literal["bytes"]
    data: str


class FileSpec(TypedDict):
    kind: Literal["file"]
    data: str
    position: int


class TextSpec(TypedDict):
    kind: Literal["text"]
    text: str


class ListSpec(TypedDict):
    kind: Literal["list"]
    items: list["MediaSpec"]


MediaSpec: TypeAlias = BytesSpec | FileSpec | TextSpec | ListSpec
Media: TypeAlias = bytes | io.BytesIO | str | list["Media"]


class AbsentMask(TypedDict):
    kind: Literal["absent"]


class NullMask(TypedDict):
    kind: Literal["null"]


class SuppliedMask(TypedDict):
    kind: Literal["supplied"]
    media: MediaSpec


MaskSpec: TypeAlias = AbsentMask | NullMask | SuppliedMask


class ImageDatumFields(TypedDict):
    url: str | None


class ImageFields(TypedDict):
    created: int
    data: list[ImageDatumFields]


IMAGES: TypeAdapter[ImageFields] = TypeAdapter(ImageFields)


@runtime_checkable
class EditConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    map_openai_params: Callable[
        [dict[str, JsonValue], str, bool],
        dict[str, JsonValue],
    ]
    validate_environment: Callable[
        [dict[str, str], str, str | None, dict[str, JsonValue] | None, str | None],
        dict[str, str],
    ]
    get_complete_url: Callable[[str, str | None, dict[str, JsonValue]], str]
    transform_image_edit_request: Callable[
        [
            str,
            str | None,
            Media | None,
            dict[str, JsonValue | Media],
            dict[str, JsonValue],
            dict[str, str],
        ],
        tuple[dict[str, JsonValue], tuple[()]],
    ]
    transform_image_edit_response: Callable[
        [str, httpx.Response, None],
        ImageResponse,
    ]


CONFIG: EditConfig = TypeAdapter(EditConfig, config=ARBITRARY).validate_python(
    BlackForestLabsImageEditConfig.__new__(BlackForestLabsImageEditConfig)
)


class PrepareCall(TypedDict):
    operation: Literal["first_image_only", "restored_file_cursor", "mask_presence"]
    prompt: str | None
    image: MediaSpec | None
    mask: MaskSpec
    options: dict[str, JsonValue | Media]


class DefaultsCall(TypedDict):
    operation: Literal["edit_defaults"]
    model: str
    drop_params: bool
    options: dict[str, JsonValue]


class EndpointCall(TypedDict):
    operation: Literal["endpoint_fill"]
    model: str
    api_base: str | None
    api_key: str | None
    environment: dict[str, str]


class SampleCall(TypedDict):
    operation: Literal["sample_required"]
    status: int
    body: str
    now: int


EditCall: TypeAlias = PrepareCall | DefaultsCall | EndpointCall | SampleCall


def media_from(spec: MediaSpec, handles: list[io.BytesIO]) -> Media:
    if spec["kind"] == "bytes":
        return base64.b64decode(spec["data"])
    if spec["kind"] == "file":
        handle = io.BytesIO(base64.b64decode(spec["data"]))
        handle.seek(spec["position"])
        handles.append(handle)
        return handle
    if spec["kind"] == "text":
        return spec["text"]
    return [media_from(item, handles) for item in spec["items"]]


def masked(
    call: PrepareCall, handles: list[io.BytesIO]
) -> dict[str, JsonValue | Media]:
    optional: dict[str, JsonValue | Media] = dict(call["options"])
    spec = call["mask"]
    if spec["kind"] == "absent":
        return optional
    if spec["kind"] == "null":
        optional["mask"] = None
        return optional
    optional["mask"] = media_from(spec["media"], handles)
    return optional


def prepared(call: PrepareCall) -> JsonValue:
    handles: list[io.BytesIO] = []
    image = None if call["image"] is None else media_from(call["image"], handles)
    optional = masked(call, handles)
    body, files = CONFIG.transform_image_edit_request(
        MODEL, call["prompt"], image, optional, {}, {}
    )
    after = [handle.tell() for handle in handles]
    answer: dict[str, JsonValue] = {
        "data": JSON.validate_python(body),
        "files": len(files),
        "positions": JSON.validate_python(after),
    }
    return JSON.validate_python(answer)


def defaults(call: DefaultsCall) -> JsonValue:
    supported = JSON.validate_python(CONFIG.get_supported_openai_params(call["model"]))
    mapped = JSON.validate_python(
        CONFIG.map_openai_params(call["options"], call["model"], call["drop_params"])
    )
    answer: dict[str, JsonValue] = {"supported": supported, "mapped": mapped}
    return JSON.validate_python(answer)


def endpoint(call: EndpointCall) -> JsonValue:
    headers = CONFIG.validate_environment(
        {}, call["model"], call["api_key"], {}, call["api_base"]
    )
    lowered: dict[str, str] = {name.lower(): value for name, value in headers.items()}
    url = CONFIG.get_complete_url(call["model"], call["api_base"], {})
    answer: dict[str, JsonValue] = {
        "headers": JSON.validate_python(lowered),
        "url": url,
    }
    return JSON.validate_python(answer)


def sample(call: SampleCall) -> JsonValue:
    raw_response = httpx.Response(call["status"], content=call["body"].encode())
    response = CONFIG.transform_image_edit_response(MODEL, raw_response, None)
    dumped = IMAGES.validate_json(response.model_dump_json())
    entries: list[JsonValue] = []
    for image in dumped["data"]:
        entry: dict[str, JsonValue] = {"url": image["url"]}
        entries.append(entry)
    answer: dict[str, JsonValue] = {"created": dumped["created"], "data": entries}
    return JSON.validate_python(answer)


def run(input_value: EditCall) -> JsonValue:
    match input_value["operation"]:
        case "first_image_only":
            return prepared(input_value)
        case "restored_file_cursor":
            return prepared(input_value)
        case "mask_presence":
            return prepared(input_value)
        case "edit_defaults":
            return defaults(input_value)
        case "endpoint_fill":
            return endpoint(input_value)
        case "sample_required":
            return sample(input_value)
