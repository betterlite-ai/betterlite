import base64
import io
import json
import pathlib
from collections.abc import Callable
from typing import Annotated, Final, Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.vertex_ai.image_edit.vertex_gemini_transformation import (
    VertexAIGeminiImageEditConfig,
)
from litellm.types.utils import ImageResponse
from pydantic import ConfigDict, Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
ARBITRARY_TYPES: Final = ConfigDict(arbitrary_types_allowed=True)
DROP_PARAMS: Final[bool] = False

# the file records LiteLLM's edit reader handles, and one it refuses
Stored: TypeAlias = bytes | io.BytesIO | io.BufferedReader | pathlib.Path
ImageInput: TypeAlias = Stored | list[Stored | None] | None


@runtime_checkable
class GeminiEditConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    map_openai_params: Callable[
        [dict[str, JsonValue], str, bool],
        dict[str, JsonValue],
    ]
    get_complete_url: Callable[[str, str | None, dict[str, JsonValue]], str]
    validate_environment: Callable[
        [dict[str, str], str, None, dict[str, JsonValue], str | None],
        dict[str, JsonValue],
    ]
    transform_image_edit_request: Callable[
        [
            str,
            str | None,
            ImageInput,
            dict[str, JsonValue],
            dict[str, JsonValue],
            dict[str, str],
        ],
        tuple[str, list[JsonValue]],
    ]
    transform_image_edit_response: Callable[
        [str, httpx.Response, None],
        ImageResponse,
    ]


# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[GeminiEditConfig] = TypeAdapter[GeminiEditConfig](
    GeminiEditConfig, config=ARBITRARY_TYPES
).validate_python(VertexAIGeminiImageEditConfig())


class BytesCase(TypedDict):
    kind: Literal["bytes"]
    data_base64: str


class SeekableCase(TypedDict):
    kind: Literal["seekable"]
    data_base64: str
    position: int


class UnsupportedCase(TypedDict):
    kind: Literal["unsupported"]


ImageCase: TypeAlias = Annotated[
    BytesCase | SeekableCase | UnsupportedCase,
    Field(discriminator="kind"),
]


class TargetCase(TypedDict):
    api_base: str | None
    params_api_base: str | None
    headers: dict[str, str]


class Input(TypedDict):
    model: str
    prompt: str | None
    image: list[ImageCase | None] | None
    optional_params: JsonObject
    target: TargetCase | None
    response: JsonObject | None


class EditDatum(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class EditReply(TypedDict):
    data: list[EditDatum]


REPLY: Final[TypeAdapter[EditReply]] = TypeAdapter(EditReply)


class PreparedImage(TypedDict):
    record: Stored
    cursor: io.BytesIO | None


def prepared_image(spec: ImageCase) -> PreparedImage:
    match spec["kind"]:
        case "bytes":
            return {"record": base64.b64decode(spec["data_base64"]), "cursor": None}
        case "seekable":
            stream = io.BytesIO(base64.b64decode(spec["data_base64"]))
            stream.seek(spec["position"])
            return {"record": stream, "cursor": stream}
        case "unsupported":
            return {"record": pathlib.Path("/images/input.png"), "cursor": None}


def prepared_images(specs: list[ImageCase | None] | None) -> list[PreparedImage | None]:
    if specs is None:
        return []
    return [None if spec is None else prepared_image(spec) for spec in specs]


def images_value(
    specs: list[ImageCase | None] | None, prepared: list[PreparedImage | None]
) -> list[Stored | None] | None:
    if specs is None:
        return None
    return [None if item is None else item["record"] for item in prepared]


# the cursor each record reads from after the transform; only a seekable record has one
def positions_value(prepared: list[PreparedImage | None]) -> list[JsonValue]:
    positions: list[JsonValue] = []
    for item in prepared:
        if item is None:
            positions.append(None)
            continue
        cursor = item["cursor"]
        positions.append(None if cursor is None else cursor.tell())
    return positions


def target_value(model: str, target: TargetCase) -> JsonObject:
    api_base = target["api_base"]
    params: JsonObject = {}
    if target["params_api_base"] is not None:
        params["api_base"] = target["params_api_base"]
    url = CONFIG.get_complete_url(model, api_base, params)
    headers = CONFIG.validate_environment(dict(target["headers"]), model, None, params, api_base)
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
    return {"data": data}


def run(input_value: Input) -> JsonValue:
    model = input_value["model"]
    supported: JsonValue = JSON.validate_python(CONFIG.get_supported_openai_params(model))
    mapped = CONFIG.map_openai_params(input_value["optional_params"], model, DROP_PARAMS)
    prepared = prepared_images(input_value["image"])
    images = images_value(input_value["image"], prepared)
    payload, files = CONFIG.transform_image_edit_request(
        model, input_value["prompt"], images, mapped, {}, {}
    )
    target = input_value["target"]
    response = input_value["response"]
    answer: JsonObject = {
        "supported": supported,
        "mapped": mapped,
        "payload": JSON.validate_json(payload),
        "files": len(files),
        "positions": positions_value(prepared),
        "target": None if target is None else target_value(model, target),
        "response": None if response is None else reply_value(model, response),
    }
    return JSON.validate_python(answer)
