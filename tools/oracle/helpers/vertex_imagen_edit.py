import base64
import json
from collections.abc import Callable, Mapping
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.constants import DEFAULT_MAX_RECURSE_DEPTH
from litellm.llms.vertex_ai.image_edit.vertex_imagen_transformation import (
    VertexAIImagenImageEditConfig,
)
from litellm.types.utils import ImageResponse
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False

# the file records LiteLLM's edit reader walks, the nested lists and wrappers among them
Stored: TypeAlias = bytes | str | list["Stored | None"] | dict[str, str]
ImageInput: TypeAlias = list[Stored | None] | None
# the transform reads the mapped JSON and the mask's file record from the same params mapping
EditParams: TypeAlias = Mapping[str, JsonValue | Stored]


@runtime_checkable
class ImagenEditConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    map_openai_params: Callable[
        [dict[str, JsonValue], str, bool],
        dict[str, JsonValue],
    ]
    get_complete_url: Callable[[str, str | None, Mapping[str, JsonValue]], str]
    validate_environment: Callable[
        [dict[str, str], str, None, Mapping[str, JsonValue], str | None],
        dict[str, JsonValue],
    ]
    transform_image_edit_request: Callable[
        [str, str | None, ImageInput, EditParams, EditParams, dict[str, str]],
        tuple[str, list[JsonValue]],
    ]
    transform_image_edit_response: Callable[
        [str, httpx.Response, None],
        ImageResponse,
    ]


# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[ImagenEditConfig] = TypeAdapter[ImagenEditConfig](
    ImagenEditConfig, config=ARBITRARY_TYPES
).validate_python(VertexAIImagenImageEditConfig())


class BytesCase(TypedDict):
    kind: Literal["bytes"]
    data_base64: str


class TextCase(TypedDict):
    kind: Literal["text"]
    text: str


class SequenceCase(TypedDict):
    kind: Literal["sequence"]
    items: list["ImageCase | None"]


class MappingCase(TypedDict):
    kind: Literal["mapping"]
    data: str | None
    bytes: str | None
    content: str | None
    path: str | None


ImageCase: TypeAlias = BytesCase | TextCase | SequenceCase | MappingCase


class TargetCase(TypedDict):
    api_base: str | None
    params_api_base: str | None
    project: str | None
    location: str | None
    headers: dict[str, str]


class Input(TypedDict):
    model: str
    optional_params: JsonObject
    prompt: NotRequired[str | None]
    image: NotRequired[list[ImageCase | None] | None]
    mask: NotRequired[ImageCase | None]
    target: NotRequired[TargetCase | None]
    response: NotRequired[JsonObject | None]
    max_recurse_depth: NotRequired[int | None]


class EditDatum(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class EditUsageDetails(TypedDict):
    image_tokens: int
    text_tokens: int


class EditUsage(TypedDict):
    input_tokens: int
    input_tokens_details: EditUsageDetails
    output_tokens: int
    total_tokens: int


class EditReply(TypedDict):
    created: int
    data: list[EditDatum]
    usage: EditUsage


REPLY: Final[TypeAdapter[EditReply]] = TypeAdapter(EditReply)


def record(spec: ImageCase) -> Stored:
    match spec["kind"]:
        case "bytes":
            return base64.b64decode(spec["data_base64"])
        case "text":
            return spec["text"]
        case "sequence":
            return [None if item is None else record(item) for item in spec["items"]]
        case "mapping":
            fields: dict[str, str] = {}
            if spec["data"] is not None:
                fields["data"] = spec["data"]
            if spec["bytes"] is not None:
                fields["bytes"] = spec["bytes"]
            if spec["content"] is not None:
                fields["content"] = spec["content"]
            if spec["path"] is not None:
                fields["path"] = spec["path"]
            return fields


def image_records(specs: list[ImageCase | None] | None) -> ImageInput:
    if specs is None:
        return None
    return [None if spec is None else record(spec) for spec in specs]


def target_value(model: str, target: TargetCase) -> JsonObject:
    params: dict[str, JsonValue] = {}
    if target["project"] is not None:
        params["vertex_project"] = target["project"]
    if target["location"] is not None:
        params["vertex_location"] = target["location"]
    if target["params_api_base"] is not None:
        params["api_base"] = target["params_api_base"]
    url = CONFIG.get_complete_url(model, target["api_base"], params)
    headers = CONFIG.validate_environment(
        dict(target["headers"]), model, None, params, target["api_base"]
    )
    return {"url": url, "headers": headers}


def datum_value(datum: EditDatum) -> JsonObject:
    value: JsonObject = {}
    if datum["b64_json"] is not None:
        value["b64_json"] = datum["b64_json"]
    return value


def reply_value(model: str, reply: JsonObject) -> JsonObject:
    raw = httpx.Response(200, content=json.dumps(reply))
    finished = CONFIG.transform_image_edit_response(model, raw, None)
    parsed = REPLY.validate_json(finished.model_dump_json())
    data: list[JsonValue] = [datum_value(datum) for datum in parsed["data"]]
    details: Final = parsed["usage"]["input_tokens_details"]
    usage: JsonObject = {
        "input_tokens": parsed["usage"]["input_tokens"],
        "input_tokens_details": {
            "image_tokens": details["image_tokens"],
            "text_tokens": details["text_tokens"],
        },
        "output_tokens": parsed["usage"]["output_tokens"],
        "total_tokens": parsed["usage"]["total_tokens"],
    }
    return {"created": parsed["created"], "data": data, "usage": usage}


def run(input_value: Input) -> JsonValue:
    model = input_value["model"]
    depth = input_value.get("max_recurse_depth")
    if depth is not None and depth != DEFAULT_MAX_RECURSE_DEPTH:
        raise ValueError("the case's depth limit and its DEFAULT_MAX_RECURSE_DEPTH disagree")
    supported: JsonValue = JSON.validate_python(CONFIG.get_supported_openai_params(model))
    mapped = CONFIG.map_openai_params(input_value["optional_params"], model, DROP_PARAMS)
    params: dict[str, JsonValue | Stored] = {**mapped}
    mask = input_value.get("mask")
    if mask is not None:
        params["mask"] = record(mask)
    payload, files = CONFIG.transform_image_edit_request(
        model,
        input_value.get("prompt"),
        image_records(input_value.get("image")),
        params,
        {},
        {},
    )
    target = input_value.get("target")
    response = input_value.get("response")
    answer: JsonObject = {
        "supported": supported,
        "mapped": mapped,
        "payload": JSON.validate_json(payload),
        "files": len(files),
        "target": None if target is None else target_value(model, target),
        "response": None if response is None else reply_value(model, response),
    }
    return JSON.validate_python(answer)
