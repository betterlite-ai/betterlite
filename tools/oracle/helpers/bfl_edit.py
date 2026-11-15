import base64
import io
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
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        image_edit_optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        api_base: str | None = None,
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def transform_image_edit_request(
        self,
        model: str,
        prompt: str | None,
        image: Media | None,
        params: dict[str, JsonValue | Media],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> tuple[dict[str, JsonValue], tuple[()]]: ...

    def transform_image_edit_response(
        self,
        model: str,
        raw_response: httpx.Response,
        logging_obj: None,
    ) -> ImageResponse: ...


CONFIG: EditConfig = TypeAdapter(EditConfig, config=ARBITRARY).validate_python(
    BlackForestLabsImageEditConfig()
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


def media_from(spec: MediaSpec) -> Media:
    if spec["kind"] == "bytes":
        return base64.b64decode(spec["data"])
    if spec["kind"] == "file":
        handle = io.BytesIO(base64.b64decode(spec["data"]))
        handle.seek(spec["position"])
        return handle
    if spec["kind"] == "text":
        return spec["text"]
    return [media_from(item) for item in spec["items"]]


def position_list(media: Media | None) -> list[int]:
    match media:
        case None:
            return []
        case io.BytesIO():
            return [media.tell()]
        case list():
            return [at for item in media for at in position_list(item)]
        case bytes() | str():
            return []


def masked(call: PrepareCall) -> tuple[dict[str, JsonValue | Media], Media | None]:
    optional: dict[str, JsonValue | Media] = dict(call["options"])
    spec = call["mask"]
    if spec["kind"] == "absent":
        return optional, None
    if spec["kind"] == "null":
        optional["mask"] = None
        return optional, None
    mask = media_from(spec["media"])
    optional["mask"] = mask
    return optional, mask


def prepared(call: PrepareCall) -> JsonValue:
    image = None if call["image"] is None else media_from(call["image"])
    optional, mask = masked(call)
    body, files = CONFIG.transform_image_edit_request(
        MODEL, call["prompt"], image, optional, {}, {}
    )
    after = [*position_list(image), *position_list(mask)]
    answer: dict[str, JsonValue] = {
        "data": JSON.validate_python(body),
        "files": len(files),
        "positions": JSON.validate_python(after),
    }
    return JSON.validate_python(answer)


def defaults(call: DefaultsCall) -> JsonValue:
    supported = JSON.validate_python(CONFIG.get_supported_openai_params(call["model"]))
    mapped = JSON.validate_python(
        CONFIG.map_openai_params(
            call["options"], call["model"], drop_params=call["drop_params"]
        )
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
    entries: list[JsonValue] = [
        JSON.validate_python({"url": image["url"]}) for image in dumped["data"]
    ]
    answer: dict[str, JsonValue] = {"created": dumped["created"], "data": entries}
    return JSON.validate_python(answer)


def run(input_value: EditCall) -> JsonValue:
    match input_value["operation"]:
        case "first_image_only" | "restored_file_cursor" | "mask_presence":
            return prepared(input_value)
        case "edit_defaults":
            return defaults(input_value)
        case "endpoint_fill":
            return endpoint(input_value)
        case "sample_required":
            return sample(input_value)
