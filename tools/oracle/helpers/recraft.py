import base64
import io
from collections.abc import Callable, Sequence
from typing import Literal, Protocol, TypedDict, cast

import httpx
from litellm.llms.recraft.cost_calculator import cost_calculator
from litellm.llms.recraft.image_edit.transformation import RecraftImageEditConfig
from litellm.llms.recraft.image_generation import get_recraft_image_generation_config
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageObject, ImageResponse, ModelInfo
from openai.types.image import Image
from pydantic import JsonValue, TypeAdapter

MODEL = "recraft/recraftv3"
EMPTY: dict[str, JsonValue] = {}
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ImageFields(TypedDict, total=False):
    b64_json: str
    url: str
    revised_prompt: str


class ResponseFields(TypedDict):
    created: int
    data: list[ImageFields]


class InputTokensFields(TypedDict):
    image_tokens: int
    text_tokens: int


class UsageFields(TypedDict):
    input_tokens: int
    input_tokens_details: InputTokensFields
    output_tokens: int
    total_tokens: int


class ReplyFields(TypedDict):
    data: list[ImageFields]
    usage: UsageFields


RESPONSE: TypeAdapter[ResponseFields] = TypeAdapter(ResponseFields)
REPLY: TypeAdapter[ReplyFields] = TypeAdapter(ReplyFields)


class Readable(Protocol):
    def read(self) -> bytes: ...


class NamedSource(io.BytesIO):
    name: str

    def __init__(self, name: str, data: bytes) -> None:
        super().__init__(data)
        self.name = name


class GenerationConfig(Protocol):
    get_supported_openai_params: Callable[[str], Sequence[str]]
    map_openai_params: Callable[
        [dict[str, JsonValue], dict[str, JsonValue], str, bool],
        dict[str, JsonValue],
    ]
    get_complete_url: Callable[
        [str | None, str | None, str, dict[str, JsonValue], dict[str, JsonValue], bool | None],
        str,
    ]
    validate_environment: Callable[
        [
            dict[str, str],
            str,
            list[JsonValue],
            dict[str, JsonValue],
            dict[str, JsonValue],
            str | None,
            str | None,
        ],
        dict[str, JsonValue],
    ]
    transform_image_generation_request: Callable[  # ast-grep-ignore: no-long-names
        [str, str, dict[str, JsonValue], dict[str, JsonValue], dict[str, str]],
        dict[str, JsonValue],
    ]
    transform_image_generation_response: Callable[  # ast-grep-ignore: no-long-names
        [
            str,
            httpx.Response,
            ImageResponse,
            JsonValue,
            dict[str, JsonValue],
            dict[str, JsonValue],
            dict[str, JsonValue],
            None,
        ],
        ImageResponse,
    ]


class EditConfig(Protocol):
    get_supported_openai_params: Callable[[str], Sequence[str]]
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
            list[Readable],
            dict[str, JsonValue],
            GenericLiteLLMParams,
            dict[str, str],
        ],
        tuple[dict[str, JsonValue], list[tuple[str, tuple[str, Readable, str]]]],
    ]
    transform_image_edit_response: Callable[[str, httpx.Response, JsonValue], ImageResponse]


class SupportedCall(TypedDict):
    function: Literal["supported"]
    endpoint: Literal["generation", "edit"]
    model: str


class MapCall(TypedDict):
    function: Literal["map"]
    model: str
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool


class EditMapCall(TypedDict):
    function: Literal["edit_map"]
    model: str
    params: dict[str, JsonValue]
    drop_params: bool


class UrlCall(TypedDict):
    function: Literal["url"]
    endpoint: Literal["generation", "edit"]
    model: str
    api_base: str | None
    environment: dict[str, str]


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    endpoint: Literal["generation", "edit"]
    model: str
    api_key: str | None
    headers: dict[str, str]
    environment: dict[str, str]


class RequestCall(TypedDict):
    function: Literal["request"]
    model: str
    prompt: str
    optional_params: dict[str, JsonValue]


class FileSeed(TypedDict):
    filename: str | None
    data_base64: str


class ImageRecord(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None


class EditRequestCall(TypedDict):
    function: Literal["edit_request"]
    model: str
    prompt: str | None
    images: list[FileSeed]
    optional_params: dict[str, JsonValue]


class ResponseCall(TypedDict):
    function: Literal["response"]
    status: int
    headers: dict[str, str]
    body: str
    created: int
    data: list[ImageRecord]


class EditResponseCall(TypedDict):
    function: Literal["edit_response"]
    status: int
    headers: dict[str, str]
    body: str


class CostCall(TypedDict):
    function: Literal["cost"]
    model: str
    output_cost_per_image: float | None
    image_count: int
    other: bool


ConfigurationCall = SupportedCall | MapCall | EditMapCall | UrlCall | EnvironmentCall
PreparationCall = RequestCall | EditRequestCall
ReplyCall = ResponseCall | EditResponseCall
Call = ConfigurationCall | PreparationCall | ReplyCall | CostCall


class Input(TypedDict):
    calls: list[Call]


def load_configs(model: str) -> tuple[GenerationConfig, EditConfig]:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/recraft/image_edit/transformation.py#L24-L28
        "tuple[GenerationConfig, EditConfig]",
        (get_recraft_image_generation_config(model), RecraftImageEditConfig()),
    )


def supported_call(call: SupportedCall) -> JsonValue:
    generation, edit = load_configs(call["model"])
    match call["endpoint"]:
        case "generation":
            return [str(name) for name in generation.get_supported_openai_params(call["model"])]
        case "edit":
            return [str(name) for name in edit.get_supported_openai_params(call["model"])]


def map_call(call: MapCall) -> JsonValue:
    generation, _edit = load_configs(call["model"])
    try:
        mapped = generation.map_openai_params(
            call["non_default_params"],
            call["optional_params"],
            call["model"],
            call["drop_params"],
        )
    except ValueError:
        return {"unsupported": True}
    return {"mapped": mapped}


def edit_map_call(call: EditMapCall) -> JsonValue:
    _generation, edit = load_configs(call["model"])
    mapped = edit.map_openai_params(call["params"], call["model"], call["drop_params"])
    return {"mapped": mapped}


def url_call(call: UrlCall) -> JsonValue:
    generation, edit = load_configs(call["model"])
    match call["endpoint"]:
        case "generation":
            url = generation.get_complete_url(call["api_base"], None, call["model"], EMPTY, EMPTY, None)
            return {"url": url}
        case "edit":
            return {"url": edit.get_complete_url(call["model"], call["api_base"], EMPTY)}


def environment_call(call: EnvironmentCall) -> JsonValue:
    generation, edit = load_configs(call["model"])
    headers: dict[str, str] = dict(call["headers"])
    match call["endpoint"]:
        case "generation":
            try:
                validated = generation.validate_environment(
                    headers, call["model"], [], EMPTY, EMPTY, call["api_key"], None
                )
            except ValueError:
                return {"missing_key": True}
            return {"headers": validated}
        case "edit":
            try:
                validated = edit.validate_environment(headers, call["model"], call["api_key"], EMPTY, None)
            except ValueError:
                return {"missing_key": True}
            return {"headers": validated}


def configuration_call(call: ConfigurationCall) -> JsonValue:
    match call["function"]:
        case "supported":
            return supported_call(call)
        case "map":
            return map_call(call)
        case "edit_map":
            return edit_map_call(call)
        case "url":
            return url_call(call)
        case "environment":
            return environment_call(call)


def request_call(call: RequestCall) -> JsonValue:
    generation, _edit = load_configs(call["model"])
    request = generation.transform_image_generation_request(
        call["model"], call["prompt"], call["optional_params"], EMPTY, EMPTY
    )
    return {"request": request}


def seed_reader(seed: FileSeed) -> Readable:
    data = base64.b64decode(seed["data_base64"])
    name = seed["filename"]
    if name is None:
        return io.BytesIO(data)
    return io.BufferedReader(NamedSource(name, data))


def part_value(part: tuple[str, tuple[str, Readable, str]]) -> JsonValue:
    field, (filename, reader, content_type) = part
    return {
        "field": field,
        "filename": filename,
        "content_type": content_type,
        "content": base64.b64encode(reader.read()).decode("ascii"),
    }


def edit_request_call(call: EditRequestCall) -> JsonValue:
    _generation, edit = load_configs(call["model"])
    images = [seed_reader(seed) for seed in call["images"]]
    data, files = edit.transform_image_edit_request(
        call["model"],
        call["prompt"],
        images,
        call["optional_params"],
        GenericLiteLLMParams(),
        EMPTY,
    )
    parts: list[JsonValue] = [part_value(part) for part in files]
    return {"data": data, "files": parts}


def preparation_call(call: PreparationCall) -> JsonValue:
    match call["function"]:
        case "request":
            return request_call(call)
        case "edit_request":
            return edit_request_call(call)


def response_call(call: ResponseCall) -> JsonValue:
    generation, _edit = load_configs(MODEL)
    response = ImageResponse(created=call["created"])
    if call["data"]:
        seeded: list[Image] = [
            ImageObject(b64_json=seed["b64_json"], url=seed["url"], revised_prompt=seed["revised_prompt"])
            for seed in call["data"]
        ]
        response.data = seeded
    finished = generation.transform_image_generation_response(
        MODEL,
        httpx.Response(call["status"], headers=call["headers"], content=call["body"]),
        response,
        None,
        EMPTY,
        EMPTY,
        EMPTY,
        None,
    )
    return {"response": JSON.validate_python(RESPONSE.validate_json(finished.model_dump_json(exclude_none=True)))}


def edit_response_call(call: EditResponseCall) -> JsonValue:
    _generation, edit = load_configs(MODEL)
    finished = edit.transform_image_edit_response(
        MODEL,
        httpx.Response(call["status"], headers=call["headers"], content=call["body"]),
        None,
    )
    return JSON.validate_python(REPLY.validate_json(finished.model_dump_json(exclude_none=True)))


def reply_call(call: ReplyCall) -> JsonValue:
    match call["function"]:
        case "response":
            return response_call(call)
        case "edit_response":
            return edit_response_call(call)


def cost_call(call: CostCall) -> JsonValue:
    model_info: ModelInfo = ModelInfo(
        key=call["model"],
        max_tokens=None,
        max_input_tokens=None,
        max_output_tokens=None,
        input_cost_per_token=0.0,
        output_cost_per_token=0.0,
        litellm_provider="recraft",
        mode="image_generation",
        supported_openai_params=[],
        output_cost_per_image=call["output_cost_per_image"],
    )
    if call["other"]:
        return {"cost": format(cost_calculator(call["model"], "other", model_info), "g")}
    images: list[Image] = [
        ImageObject(b64_json=None, url=f"https://recraft.test/{index}.png", revised_prompt=None)
        for index in range(call["image_count"])
    ]
    response = ImageResponse(created=1)
    response.data = images
    return {"cost": format(cost_calculator(call["model"], response, model_info), "g")}


def call_value(call: Call) -> JsonValue:
    match call["function"]:
        case "supported" | "map" | "edit_map" | "url" | "environment":
            return configuration_call(call)
        case "request" | "edit_request":
            return preparation_call(call)
        case "response" | "edit_response":
            return reply_call(call)
        case "cost":
            return cost_call(call)


def run(input_value: Input) -> JsonValue:
    calls = input_value["calls"]
    return [call_value(call) for call in calls]
