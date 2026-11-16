import base64
import io
from collections.abc import Callable, Sequence
from typing import Literal, Protocol, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.recraft.cost_calculator import cost_calculator
from litellm.llms.recraft.image_edit.transformation import RecraftImageEditConfig
from litellm.llms.recraft.image_generation import get_recraft_image_generation_config as config_for
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ImageObject, ImageResponse
from openai.types.image import Image
from pydantic import JsonValue, TypeAdapter

MODEL = "recraft/recraftv3"
Obj: TypeAlias = dict[str, JsonValue]
EMPTY: Obj = {}
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
CostFunction: TypeAlias = Callable[[str, ImageResponse | str, Obj], float]


class Reader(Protocol):
    def read(self) -> bytes: ...


class NamedSource(io.BytesIO):
    name: str

    def __init__(self, name: str, data: bytes) -> None:
        super().__init__(data)
        self.name = name


class GenerationConfig(Protocol):
    get_supported_openai_params: Callable[[str], Sequence[str]]
    map_openai_params: Callable[[Obj, Obj, str, bool], Obj]
    get_complete_url: Callable[[str | None, str | None, str, Obj, Obj, bool | None], str]
    validate_environment: Callable[
        [dict[str, str], str, list[JsonValue], Obj, Obj, str | None, str | None], Obj
    ]
    transform_image_generation_request: Callable[  # ast-grep-ignore: no-long-names
        [str, str, Obj, Obj, dict[str, str]], Obj
    ]
    transform_image_generation_response: Callable[  # ast-grep-ignore: no-long-names
        [str, httpx.Response, ImageResponse, JsonValue, Obj, Obj, Obj, None], ImageResponse
    ]


class EditConfig(Protocol):
    get_supported_openai_params: Callable[[str], Sequence[str]]
    map_openai_params: Callable[[Obj, str, bool], Obj]
    get_complete_url: Callable[[str, str | None, Obj], str]
    validate_environment: Callable[[dict[str, str], str, str | None, Obj, str | None], Obj]
    transform_image_edit_request: Callable[
        [str, str | None, list[Reader], Obj, GenericLiteLLMParams, dict[str, str]],
        tuple[Obj, list[tuple[str, tuple[str, Reader, str]]]],
    ]
    transform_image_edit_response: Callable[[str, httpx.Response, JsonValue], ImageResponse]


Providers: TypeAlias = tuple[GenerationConfig, EditConfig, CostFunction]


class SupportedCall(TypedDict):
    function: Literal["supported"]
    endpoint: Literal["generation", "edit"]
    model: str


class MapCall(TypedDict):
    function: Literal["map"]
    model: str
    non_default_params: Obj
    optional_params: Obj
    drop_params: bool


class EditMapCall(TypedDict):
    function: Literal["edit_map"]
    model: str
    params: Obj
    drop_params: bool


class UrlCall(TypedDict):
    function: Literal["url"]
    endpoint: Literal["generation", "edit"]
    model: str
    api_base: str | None


class EnvironmentCall(TypedDict):
    function: Literal["environment"]
    endpoint: Literal["generation", "edit"]
    model: str
    api_key: str | None
    headers: dict[str, str]


class RequestCall(TypedDict):
    function: Literal["request"]
    model: str
    prompt: str
    optional_params: Obj


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
    optional_params: Obj


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


ConfigurationCall: TypeAlias = SupportedCall | MapCall | EditMapCall | UrlCall | EnvironmentCall
ExchangeCall: TypeAlias = RequestCall | EditRequestCall | ResponseCall | EditResponseCall
Call: TypeAlias = ConfigurationCall | ExchangeCall | CostCall


class Input(TypedDict):
    calls: list[Call]


PROVIDERS = cast("Providers", (config_for(MODEL), RecraftImageEditConfig(), cost_calculator))  # type: ignore[no-untyped-call, misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/image_edit/transformation.py#L30
GENERATION, EDIT, COST = PROVIDERS


def environment_call(call: EnvironmentCall) -> JsonValue:
    headers = dict(call["headers"])
    validated: Obj
    try:
        match call["endpoint"]:
            case "generation":
                validated = GENERATION.validate_environment(
                    headers, call["model"], [], EMPTY, EMPTY, call["api_key"], None
                )
            case "edit":
                validated = EDIT.validate_environment(
                    headers, call["model"], call["api_key"], EMPTY, None
                )
    except ValueError:
        return {"missing_key": True}
    return {"headers": validated}


def seed_reader(seed: FileSeed) -> Reader:
    data = base64.b64decode(seed["data_base64"])
    name = seed["filename"]
    if name is None:
        return io.BytesIO(data)
    return io.BufferedReader(NamedSource(name, data))


def edit_request_call(call: EditRequestCall) -> JsonValue:
    images = [seed_reader(seed) for seed in call["images"]]
    data, files = EDIT.transform_image_edit_request(
        call["model"], call["prompt"], images, call["optional_params"], GenericLiteLLMParams(), {}
    )
    parts: list[JsonValue] = [
        {
            "field": field,
            "filename": filename,
            "content_type": content_type,
            "content": base64.b64encode(reader.read()).decode("ascii"),
        }
        for field, (filename, reader, content_type) in files
    ]
    return {"data": data, "files": parts}


def response_call(call: ResponseCall) -> JsonValue:
    response = ImageResponse(created=call["created"])
    if call["data"]:
        seeded: list[Image] = [
            ImageObject(
                b64_json=seed["b64_json"], url=seed["url"], revised_prompt=seed["revised_prompt"]
            )
            for seed in call["data"]
        ]
        response.data = seeded
    upstream = httpx.Response(call["status"], headers=call["headers"], content=call["body"])
    finished = GENERATION.transform_image_generation_response(
        MODEL, upstream, response, None, EMPTY, EMPTY, EMPTY, None
    )
    dumped = finished.model_dump_json(exclude_none=True, include={"created", "data"})
    return {"response": JSON.validate_json(dumped)}


def cost_call(call: CostCall) -> JsonValue:
    model_info: Obj = {"output_cost_per_image": call["output_cost_per_image"]}
    response: ImageResponse | str = "other"
    if not call["other"]:
        response = ImageResponse(created=1)
        response.data = [
            ImageObject(b64_json=None, url=f"https://recraft.test/{index}.png", revised_prompt=None)
            for index in range(call["image_count"])
        ]
    return {"cost": format(COST(call["model"], response, model_info), "g")}


def configuration_call(call: ConfigurationCall) -> JsonValue:
    match call["function"]:
        case "supported":
            config: GenerationConfig | EditConfig = GENERATION
            if call["endpoint"] == "edit":
                config = EDIT
            return [str(name) for name in config.get_supported_openai_params(call["model"])]
        case "map":
            try:
                mapped = GENERATION.map_openai_params(
                    call["non_default_params"],
                    call["optional_params"],
                    call["model"],
                    call["drop_params"],
                )
            except ValueError:
                return {"unsupported": True}
            return {"mapped": mapped}
        case "edit_map":
            mapped = EDIT.map_openai_params(call["params"], call["model"], call["drop_params"])
            return {"mapped": mapped}
        case "url":
            if call["endpoint"] == "edit":
                url = EDIT.get_complete_url(call["model"], call["api_base"], EMPTY)
            else:
                url = GENERATION.get_complete_url(
                    call["api_base"], None, call["model"], EMPTY, EMPTY, None
                )
            return {"url": url}
        case "environment":
            return environment_call(call)


def exchange_call(call: ExchangeCall) -> JsonValue:
    match call["function"]:
        case "request":
            request = GENERATION.transform_image_generation_request(
                call["model"], call["prompt"], call["optional_params"], EMPTY, {}
            )
            return {"request": request}
        case "edit_request":
            return edit_request_call(call)
        case "response":
            return response_call(call)
        case "edit_response":
            upstream = httpx.Response(call["status"], headers=call["headers"], content=call["body"])
            finished = EDIT.transform_image_edit_response(MODEL, upstream, None)
            dumped = finished.model_dump_json(exclude_none=True, include={"data", "usage"})
            return JSON.validate_json(dumped)


def call_value(call: Call) -> JsonValue:
    match call["function"]:
        case "supported" | "map" | "edit_map" | "url" | "environment":
            return configuration_call(call)
        case "request" | "edit_request" | "response" | "edit_response":
            return exchange_call(call)
        case "cost":
            return cost_call(call)


def run(input_value: Input) -> JsonValue:
    answers: list[JsonValue] = [call_value(call) for call in input_value["calls"]]
    return answers
