import os
from collections.abc import Callable
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.fal_ai.image_generation.bytedance_transformation import (
    FalAIBytedanceDreaminaV31Config,
    FalAIBytedanceSeedreamV3Config,
)
from litellm.llms.fal_ai.image_generation.imagen4_transformation import FalAIImagen4Config
from litellm.llms.fal_ai.image_generation.nano_banana_transformation import FalAINanoBananaConfig
from litellm.types.utils import ImageObject, ImageResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MODEL = "fal-ai/model"
Family: TypeAlias = Literal["imagen", "nano_banana", "seedream", "dreamina"]
Params: TypeAlias = dict[str, JsonValue]
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

GenerationRequest: TypeAlias = Callable[[str, str, Params, Params, dict[str, str]], Params]
GenerationResponse: TypeAlias = Callable[
    [str, httpx.Response, ImageResponse, Logging, Params, Params, Params, None], ImageResponse
]


class FamilyConfig(Protocol):
    IMAGE_GENERATION_ENDPOINT: str
    @property
    def map_openai_params(self) -> Callable[[Params, Params, str, bool], Params]: ...
    @property
    def get_complete_url(self) -> Callable[[str | None, str | None, str, Params, Params], str]: ...
    @property
    def transform_image_generation_request(  # ast-grep-ignore: no-long-names
        self
    ) -> GenerationRequest: ...
    @property
    def transform_image_generation_response(  # ast-grep-ignore: no-long-names
        self
    ) -> GenerationResponse: ...


class HiddenRecord(BaseModel):
    hidden_params: dict[str, JsonValue] = Field(alias="_hidden_params")


class ErrorRecord(BaseModel):
    status_code: int


class ProviderFields(TypedDict, total=False):
    width: int
    height: int
    content_type: str


class ImageFields(TypedDict, total=False):
    url: str
    b64_json: str
    provider_specific_fields: ProviderFields


class ExistingImage(TypedDict):
    url: str | None
    b64_json: str | None


class MappingCase(TypedDict):
    non_default: Params
    optional: Params
    drop_params: bool


class ReplyCase(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    existing: list[JsonValue]


class ImagenFixedPreview(TypedDict):
    operation: Literal["imagen_fixed_preview"]
    prompt: str
    models: list[str]
    api_bases: list[str | None]
    cases: list[MappingCase]


class RatioAlgorithmsDiffer(TypedDict):
    operation: Literal["ratio_algorithms_differ"]
    sizes: list[str]
    banana_sizes: list[JsonValue]


class BananaDestinationWins(TypedDict):
    operation: Literal["banana_destination_wins"]
    prompt: str
    models: list[str]
    api_bases: list[str | None]
    cases: list[MappingCase]


class BananaResolutionPassthrough(TypedDict):
    operation: Literal["banana_resolution_passthrough"]
    prompt: str
    cases: list[MappingCase]


class BytedanceEndpoints(TypedDict):
    operation: Literal["bytedance_endpoints"]
    prompt: str
    families: list[Family]
    api_bases: list[str | None]
    cases: list[MappingCase]


class FamilyReplyMetadata(TypedDict):
    operation: Literal["family_reply_metadata"]
    families: list[Family]
    replies: list[ReplyCase]


Input: TypeAlias = (
    ImagenFixedPreview
    | RatioAlgorithmsDiffer
    | BananaDestinationWins
    | BananaResolutionPassthrough
    | BytedanceEndpoints
    | FamilyReplyMetadata
)

RATIO_FAMILIES: Final[tuple[Family, Family]] = ("imagen", "nano_banana")
EXISTING: TypeAdapter[list[ExistingImage]] = TypeAdapter(list[ExistingImage])
IMAGE_FIELDS: TypeAdapter[ImageFields] = TypeAdapter(ImageFields)
CONFIGS: Final[dict[Family, FamilyConfig]] = {
    "imagen": FalAIImagen4Config(),
    "nano_banana": FalAINanoBananaConfig(),
    "seedream": FalAIBytedanceSeedreamV3Config(),
    "dreamina": FalAIBytedanceDreaminaV31Config(),
}


def apply_environment() -> None:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)


def make_logging() -> Logging:
    return Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="image_generation",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def url_entry(family: Family, model: str, api_base: str | None) -> dict[str, JsonValue]:
    url = CONFIGS[family].get_complete_url(api_base, None, model, {}, {})
    return {"model": model, "api_base": api_base, "url": url}


def request_outcome(family: Family, prompt: str, case: MappingCase) -> JsonValue:
    config = CONFIGS[family]
    try:
        mapped = config.map_openai_params(
            dict(case["non_default"]), dict(case["optional"]), MODEL, case["drop_params"]
        )
        body = config.transform_image_generation_request(MODEL, prompt, mapped, {}, {})
    except ValueError:
        return {"error": {"status": None}}
    return {"request": body}


def size_outcome(family: Family, size: JsonValue) -> JsonValue:
    drop_params: Final = False
    try:
        mapped = CONFIGS[family].map_openai_params({"size": size}, {}, MODEL, drop_params)
    except ValueError:
        outcome: dict[str, JsonValue] = {"error": {"status": None}}
    else:
        outcome = {"params": mapped}
    answer: dict[str, JsonValue] = {"family": family, "size": size, "outcome": outcome}
    return answer


def ratio_algorithms_differ(call: RatioAlgorithmsDiffer) -> JsonValue:
    cases: list[JsonValue] = [
        size_outcome(family, size) for size in call["sizes"] for family in RATIO_FAMILIES
    ]
    cases.extend(size_outcome("nano_banana", size) for size in call["banana_sizes"])
    answer: dict[str, JsonValue] = {"cases": cases}
    return answer


def bytedance_entry(call: BytedanceEndpoints, family: Family) -> dict[str, JsonValue]:
    return {
        "family": family,
        "urls": [url_entry(family, MODEL, api_base) for api_base in call["api_bases"]],
        "cases": [request_outcome(family, call["prompt"], case) for case in call["cases"]],
    }


def bytedance_endpoints(call: BytedanceEndpoints) -> JsonValue:
    entries: list[JsonValue] = [bytedance_entry(call, family) for family in call["families"]]
    answer: dict[str, JsonValue] = {"families": entries}
    return answer


def reply_outcome(config: FamilyConfig, reply: ReplyCase) -> JsonValue:
    model_response = ImageResponse(created=0)
    if reply["existing"]:
        model_response.data = [
            ImageObject(url=entry["url"], b64_json=entry["b64_json"])
            for entry in EXISTING.validate_python(reply["existing"])
        ]
    raw = httpx.Response(reply["status"], headers=reply["headers"], content=reply["body"])
    try:
        result = config.transform_image_generation_response(
            MODEL, raw, model_response, make_logging(), {}, {}, {}, None
        )
    except PROVIDER_ERROR as error:
        raised = ErrorRecord.model_validate(error, from_attributes=True)
        return {"error": {"status": raised.status_code}}
    images: list[JsonValue] = []
    if result.data is not None:
        images = [
            JSON.validate_python(
                IMAGE_FIELDS.validate_json(image.model_dump_json(exclude_none=True))
            )
            for image in result.data
        ]
    hidden = HiddenRecord.model_validate(result, from_attributes=True)
    answer: dict[str, JsonValue] = {"data": images, "hidden_params": hidden.hidden_params}
    return JSON.validate_python(answer)


def reply_entry(call: FamilyReplyMetadata, family: Family) -> dict[str, JsonValue]:
    replies: list[JsonValue] = [reply_outcome(CONFIGS[family], reply) for reply in call["replies"]]
    return {"family": family, "replies": replies}


def family_reply_metadata(call: FamilyReplyMetadata) -> JsonValue:
    entries: list[JsonValue] = [reply_entry(call, family) for family in call["families"]]
    answer: dict[str, JsonValue] = {"families": entries}
    return answer


def run(input_value: Input) -> JsonValue:
    apply_environment()
    if input_value["operation"] == "imagen_fixed_preview":
        answer: dict[str, JsonValue] = {
            "endpoint": CONFIGS["imagen"].IMAGE_GENERATION_ENDPOINT,
            "urls": [
                url_entry("imagen", model, api_base)
                for model in input_value["models"]
                for api_base in input_value["api_bases"]
            ],
            "cases": [
                request_outcome("imagen", input_value["prompt"], case)
                for case in input_value["cases"]
            ],
        }
        return answer
    if input_value["operation"] == "ratio_algorithms_differ":
        return ratio_algorithms_differ(input_value)
    if input_value["operation"] == "banana_destination_wins":
        banana: dict[str, JsonValue] = {
            "urls": [
                url_entry("nano_banana", model, api_base)
                for model in input_value["models"]
                for api_base in input_value["api_bases"]
            ],
            "cases": [
                request_outcome("nano_banana", input_value["prompt"], case)
                for case in input_value["cases"]
            ],
        }
        return banana
    if input_value["operation"] == "banana_resolution_passthrough":
        passthrough: dict[str, JsonValue] = {
            "cases": [
                request_outcome("nano_banana", input_value["prompt"], case)
                for case in input_value["cases"]
            ],
        }
        return passthrough
    if input_value["operation"] == "bytedance_endpoints":
        return bytedance_endpoints(input_value)
    return family_reply_metadata(input_value)
