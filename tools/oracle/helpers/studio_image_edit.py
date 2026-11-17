import base64
import io
import json
import os
import pathlib
from collections.abc import Callable
from decimal import Decimal
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.gemini.image_edit import get_gemini_image_edit_config
from litellm.llms.gemini.image_edit.cost_calculator import cost_calculator as edit_cost_calculator
from litellm.types.utils import ImageObject, ImageResponse, ImageUsage
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JsonObject = dict[str, JsonValue]
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
DROP_PARAMS: Final[bool] = False
STUDIO_VARIABLES: Final = ("GEMINI_API_KEY", "GEMINI_API_BASE", "GOOGLE_API_KEY")
# the edit response transform ignores its model argument, so one literal stands in for every case
EDIT_MODEL: Final[str] = "gemini-2.5-flash-image"

# the file records LiteLLM's edit reader handles, and one it refuses
Stored: TypeAlias = bytes | io.BytesIO | pathlib.Path
ImageInput: TypeAlias = Stored | list[Stored | None] | None


@runtime_checkable
class GeminiEditConfig(Protocol):
    get_supported_openai_params: Callable[[str], list[str]]
    use_multipart_form_data: Callable[[], bool]
    map_openai_params: Callable[[dict[str, JsonValue], str, bool], dict[str, JsonValue]]
    get_complete_url: Callable[[str, str | None, dict[str, JsonValue]], str]
    validate_environment: Callable[
        [dict[str, str], str, str | None, dict[str, JsonValue], str | None],
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
        tuple[dict[str, JsonValue], list[JsonValue]],
    ]
    transform_image_edit_response: Callable[[str, httpx.Response, None], ImageResponse]


# litellm's config classes carry untyped parameters, so the protocol is the declared shape
CONFIG: Final[GeminiEditConfig] = TypeAdapter[GeminiEditConfig](
    GeminiEditConfig, config=ConfigDict(arbitrary_types_allowed=True)
).validate_python(get_gemini_image_edit_config(EDIT_MODEL))


class BytesCase(TypedDict):
    kind: Literal["bytes"]
    data_base64: str


class SeekableCase(TypedDict):
    kind: Literal["seekable"]
    data_base64: str
    position: int


class UnsupportedCase(TypedDict):
    kind: Literal["unsupported"]


ImageCase: TypeAlias = BytesCase | SeekableCase | UnsupportedCase


class OneField(TypedDict):
    kind: Literal["one"]
    record: ImageCase


class ManyField(TypedDict):
    kind: Literal["many"]
    records: list[ImageCase | None]


ImageField: TypeAlias = OneField | ManyField


class RequestCase(TypedDict):
    model: str
    prompt: NotRequired[str | None]
    image: NotRequired[ImageField | None]
    params: NotRequired[JsonObject]
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    env: NotRequired[dict[str, str]]


class ResponseCase(TypedDict):
    body: JsonObject


class CostUsage(TypedDict):
    input_tokens: int
    output_tokens: int
    total_tokens: int
    text_tokens: NotRequired[int]
    image_tokens: NotRequired[int]


class CostCase(TypedDict):
    model: str
    catalog: JsonObject
    images: int
    usage: NotRequired[CostUsage]


class Input(TypedDict):
    factories: list[str]
    requests: list[RequestCase]
    responses: list[ResponseCase]
    costs: list[CostCase]


class EditDatum(TypedDict):
    b64_json: str | None
    provider_specific_fields: dict[str, JsonValue] | None


class EditReply(TypedDict):
    data: list[EditDatum]
    usage: JsonObject | None


REPLY: Final[TypeAdapter[EditReply]] = TypeAdapter(EditReply)


class PreparedImage(TypedDict):
    record: Stored
    cursor: io.BytesIO | None


class PreparedField(TypedDict):
    kind: Literal["one", "many"]
    records: list[PreparedImage | None]


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


def prepared_field(field: ImageField | None) -> PreparedField | None:
    if field is None:
        return None
    if field["kind"] == "one":
        return {"kind": "one", "records": [prepared_image(field["record"])]}
    return {
        "kind": "many",
        "records": [None if spec is None else prepared_image(spec) for spec in field["records"]],
    }


def images_value(prepared: PreparedField | None) -> ImageInput:
    if prepared is None:
        return None
    if prepared["kind"] == "one":
        record = prepared["records"][0]
        return None if record is None else record["record"]
    return [None if item is None else item["record"] for item in prepared["records"]]


def cursor_position(prepared: PreparedImage | None) -> JsonValue:
    if prepared is None:
        return None
    cursor = prepared["cursor"]
    return None if cursor is None else cursor.tell()


def positions_value(prepared: PreparedField | None) -> list[JsonValue]:
    if prepared is None:
        return []
    return [cursor_position(item) for item in prepared["records"]]


def factory_value(model: str) -> JsonObject:
    config = get_gemini_image_edit_config(model)
    return {"model": model, "config": type(config).__name__}


def request_value(case: RequestCase) -> JsonObject:
    # only the three Studio variables change, so dispatch's cost-map setting stays in place
    for name in STUDIO_VARIABLES:
        os.environ.pop(name, None)
    env = case.get("env")
    if env is not None:
        os.environ.update(env)
    model = case["model"]
    params = case.get("params")
    mapped = CONFIG.map_openai_params({} if params is None else params, model, DROP_PARAMS)
    supported: JsonValue = JSON.validate_python(CONFIG.get_supported_openai_params(model))
    api_base = case.get("api_base")
    headers = case.get("headers")
    outgoing = CONFIG.validate_environment(
        {} if headers is None else dict(headers),
        model,
        case.get("api_key"),
        {},
        api_base,
    )
    prepared = prepared_field(case.get("image"))
    payload, files = CONFIG.transform_image_edit_request(
        model, case.get("prompt"), images_value(prepared), mapped, {}, {}
    )
    return {
        "url": CONFIG.get_complete_url(model, api_base, {}),
        "headers": outgoing,
        "supported": supported,
        "multipart": CONFIG.use_multipart_form_data(),
        "mapped": mapped,
        "payload": payload,
        "files": len(files),
        "positions": positions_value(prepared),
    }


def image_value(datum: EditDatum) -> JsonObject:
    fields = datum["provider_specific_fields"]
    signature = None if fields is None else fields.get("thought_signature")
    return {"b64_json": datum["b64_json"], "thought_signature": signature}


def response_value(case: ResponseCase) -> JsonObject:
    raw = httpx.Response(200, content=json.dumps(case["body"]))
    finished = CONFIG.transform_image_edit_response(EDIT_MODEL, raw, None)
    # the transform builds its own ImageResponse, whose constructor seeds a zero usage
    if "usageMetadata" not in case["body"]:
        finished.usage = None
    parsed = REPLY.validate_json(finished.model_dump_json())
    usage = parsed["usage"]
    return {
        "images": [image_value(datum) for datum in parsed["data"]],
        "usage": None if usage is None else JSON.validate_python(usage),
    }


def cost_value(case: CostCase) -> JsonObject:
    entry: JsonObject = {
        **case["catalog"],
        "litellm_provider": "gemini",
        "mode": "image_generation",
    }
    pricing: dict[str, dict[str, JsonValue]] = {f"gemini/{case['model']}": entry}
    litellm.register_model(pricing)
    response = ImageResponse(data=[ImageObject(b64_json="AAA") for _ in range(case["images"])])
    usage = case.get("usage")
    if usage is None:
        response.usage = None
    else:
        image_tokens = usage.get("image_tokens")
        text_tokens = usage.get("text_tokens")
        response.usage = ImageUsage.model_validate(
            {
                "input_tokens": usage["input_tokens"],
                "input_tokens_details": {
                    "image_tokens": 0 if image_tokens is None else image_tokens,
                    "text_tokens": 0 if text_tokens is None else text_tokens,
                },
                "output_tokens": usage["output_tokens"],
                "total_tokens": usage["total_tokens"],
            }
        )
    cost = edit_cost_calculator(case["model"], response)
    return {"cost": format(Decimal(repr(cost)).normalize(), "f")}


def run(input_value: Input) -> JsonValue:
    answer: JsonObject = {
        "factories": [factory_value(model) for model in input_value["factories"]],
        "requests": [request_value(case) for case in input_value["requests"]],
        "responses": [response_value(case) for case in input_value["responses"]],
        "costs": [cost_value(case) for case in input_value["costs"]],
    }
    return JSON.validate_python(answer)
