from collections.abc import Sequence
from datetime import UTC, datetime
from decimal import Decimal
from typing import Literal, Protocol, TypedDict

import httpx
import litellm
import litellm.llms.runwayml.image_generation.transformation as runway_transformation
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.runwayml.cost_calculator import cost_calculator
from litellm.llms.runwayml.image_generation import get_runwayml_image_generation_config
from litellm.types.utils import ImageObject, ImageResponse, ModelInfo
from openai.types.image import Image
from pydantic import ConfigDict, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EMPTY: dict[str, JsonValue] = {}
CLIENT_KEY = "httpx_client"
SUBMISSION_URL = "https://submission.example.test/v1/text_to_image"
PROVIDER = "runwayml"


class ClientCache(Protocol):
    def set_cache(self, key: str, value: HTTPHandler, *, ttl: int) -> None: ...


CLIENT_CACHE: TypeAdapter[ClientCache] = TypeAdapter(
    ClientCache, config=ConfigDict(arbitrary_types_allowed=True)
)


class ImageConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> Sequence[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def transform_image_generation_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        prompt: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> dict[str, JsonValue]: ...

    def transform_image_generation_response(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ImageResponse,
        logging_obj: Logging,
        request_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
    ) -> ImageResponse: ...


class ImageSeed(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None


class MappingCall(TypedDict):
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: bool


class MappingParams(TypedDict):
    model: str
    prompt: str
    calls: list[MappingCall]


class MappingInput(TypedDict):
    operation: Literal["mapping"]
    params: MappingParams


class Submission(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Reply(TypedDict):
    status: int
    body: str


class ResponseParams(TypedDict):
    model: str
    environment: dict[str, str]
    submission: Submission
    request_headers: dict[str, str]
    created: int
    data: list[ImageSeed]
    step_millis: int
    replies: list[Reply]


class ResponseInput(TypedDict):
    operation: Literal["response"]
    params: ResponseParams


class Prices(TypedDict, total=False):
    output_cost_per_image: float | None


class CostCall(TypedDict):
    data: list[ImageSeed]
    prices: Prices


class CostParams(TypedDict):
    model: str
    calls: list[CostCall]


class CostInput(TypedDict):
    operation: Literal["cost"]
    params: CostParams


Input = MappingInput | ResponseInput | CostInput


class Clock:
    def __init__(self, step_millis: int) -> None:
        self.millis = 0
        self.step_millis = step_millis
        self.sleeps: list[JsonValue] = []

    def time(self) -> float:
        current = self.millis
        self.millis += self.step_millis
        return current / 1000.0

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(int(seconds))


class Scripted:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[httpx.Request] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if not self.replies:
            return httpx.Response(500, content=b"")
        reply = self.replies.pop(0)
        return httpx.Response(reply["status"], content=reply["body"].encode())


def config_for(model: str) -> ImageConfig:
    config: ImageConfig = get_runwayml_image_generation_config(model)
    return config


def seeded_images(seeds: list[ImageSeed]) -> list[Image]:
    images: list[Image] = [
        ImageObject(
            b64_json=seed["b64_json"],
            url=seed["url"],
            revised_prompt=seed["revised_prompt"],
        )
        for seed in seeds
    ]
    return images


def image_value(image: Image) -> JsonValue:
    entry: dict[str, JsonValue] = {}
    if image.b64_json is not None:
        entry["b64_json"] = image.b64_json
    if image.url is not None:
        entry["url"] = image.url
    if image.revised_prompt is not None:
        entry["revised_prompt"] = image.revised_prompt
    return JSON.validate_python(entry)


def response_value(finished: ImageResponse) -> JsonValue:
    data = finished.data
    if data is None:
        return JSON.validate_python({"created": finished.created, "data": []})
    images: list[JsonValue] = [image_value(image) for image in data]
    answer: dict[str, JsonValue] = {"created": finished.created, "data": images}
    return JSON.validate_python(answer)


def request_value(request: httpx.Request) -> JsonValue:
    headers: dict[str, JsonValue] = {
        "authorization": request.headers["authorization"],
        "x-runway-version": request.headers["x-runway-version"],
    }
    answer: dict[str, JsonValue] = {"url": str(request.url), "headers": headers}
    return JSON.validate_python(answer)


def logging_object(model: str) -> Logging:
    return Logging(
        model=model,
        messages=[],
        stream=False,
        call_type="image_generation",
        start_time=datetime.now(UTC),
        litellm_call_id="runway-image-call",
        function_id="runway-image-function",
    )


def install_client(scripted: Scripted) -> None:
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    cache = CLIENT_CACHE.validate_python(holder)
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(scripted)))
    cache.set_cache(key=CLIENT_KEY, value=client, ttl=600)


def install_clock(step_millis: int) -> Clock:
    clock = Clock(step_millis)
    object.__setattr__(runway_transformation, "time", clock)
    return clock


def mapping_value(input_value: MappingInput) -> JsonValue:
    params = input_value["params"]
    config = config_for(params["model"])
    results: list[JsonValue] = []
    for call in params["calls"]:
        try:
            mapped = config.map_openai_params(
                call["non_default_params"],
                call["optional_params"],
                params["model"],
                call["drop_params"],
            )
        except ValueError:
            results.append({"unsupported": True})
            continue
        body = config.transform_image_generation_request(
            params["model"], params["prompt"], mapped, EMPTY, EMPTY
        )
        results.append({"params": mapped, "body": body})
    supported = JSON.validate_python(
        list(config.get_supported_openai_params(params["model"]))
    )
    answer: dict[str, JsonValue] = {"supported": supported, "results": results}
    return JSON.validate_python(answer)


def transform_value(input_value: ResponseInput) -> JsonValue:
    params = input_value["params"]
    config = config_for(params["model"])
    scripted = Scripted(params["replies"])
    install_client(scripted)
    clock = install_clock(params["step_millis"])
    model_response = ImageResponse(created=params["created"])
    if params["data"]:
        model_response.data = seeded_images(params["data"])
    request = httpx.Request("POST", SUBMISSION_URL, headers=params["request_headers"])
    submission = params["submission"]
    raw_response = httpx.Response(
        submission["status"],
        headers=submission["headers"],
        content=submission["body"].encode(),
        request=request,
    )
    finished = config.transform_image_generation_response(
        params["model"],
        raw_response,
        model_response,
        logging_object(params["model"]),
        EMPTY,
        EMPTY,
        EMPTY,
        None,
    )
    gets: list[JsonValue] = [request_value(item) for item in scripted.requests]
    answer: dict[str, JsonValue] = {
        "response": response_value(finished),
        "gets": gets,
        "sleeps": clock.sleeps,
    }
    return JSON.validate_python(answer)


def model_info_for(model: str, prices: Prices) -> ModelInfo:
    info = ModelInfo(
        key=model,
        max_tokens=0,
        max_input_tokens=0,
        max_output_tokens=0,
        input_cost_per_token=0.0,
        output_cost_per_token=0.0,
        litellm_provider=PROVIDER,
        mode="image_generation",
        supported_openai_params=["size"],
    )
    if "output_cost_per_image" in prices:
        info["output_cost_per_image"] = prices["output_cost_per_image"]
    return info


def cost_value(input_value: CostInput) -> JsonValue:
    params = input_value["params"]
    results: list[JsonValue] = []
    for call in params["calls"]:
        model_response = ImageResponse(created=1)
        model_response.data = seeded_images(call["data"])
        cost = cost_calculator(
            model=params["model"],
            image_response=model_response,
            model_info=model_info_for(params["model"], call["prices"]),
        )
        results.append({"cost": format(Decimal(repr(cost)).normalize(), "f")})
    answer: dict[str, JsonValue] = {"costs": results}
    return JSON.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "mapping":
        return mapping_value(input_value)
    if input_value["operation"] == "response":
        return transform_value(input_value)
    return cost_value(input_value)
