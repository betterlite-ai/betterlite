import os
from collections.abc import Callable, Sequence
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.fal_ai.image_generation.flux_dev_transformation import FalAIFluxDevConfig
from litellm.llms.fal_ai.image_generation.flux_pro_v11_transformation import (
    FalAIFluxProV11Config,
)
from litellm.llms.fal_ai.image_generation.flux_pro_v11_ultra_transformation import (
    FalAIFluxProV11UltraConfig,
)
from litellm.llms.fal_ai.image_generation.flux_schnell_transformation import (
    FalAIFluxSchnellConfig,
)
from litellm.types.utils import ImageObject, ImageResponse
from openai.types.images_response import Image
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MODEL = "fal-ai/model"
DROP_PARAMS: Final[bool] = False
Profile: TypeAlias = Literal["ultra", "pro_v11", "schnell", "dev"]
Params: TypeAlias = dict[str, JsonValue]


class FluxConfig(Protocol):
    IMAGE_GENERATION_ENDPOINT: str
    get_supported_openai_params: Callable[[str], Sequence[str]]
    map_openai_params: Callable[[Params, Params, str, bool], Params]
    get_complete_url: Callable[[str | None, str | None, str, Params, Params], str]
    transform_image_generation_request: Callable[  # ast-grep-ignore: no-long-names
        [str, str, Params, Params, dict[str, str]], Params
    ]
    transform_image_generation_response: Callable[  # ast-grep-ignore: no-long-names
        [str, httpx.Response, ImageResponse, Logging, Params, Params, Params, None],
        ImageResponse,
    ]


class HiddenRecord(BaseModel):
    hidden_params: dict[str, JsonValue] = Field(alias="_hidden_params")


class ErrorRecord(BaseModel):
    status_code: int


class ProviderFields(TypedDict, total=False):
    width: int
    height: int
    content_type: str


class ImageFields(TypedDict, total=False):
    url: str | None
    b64_json: str | None
    provider_specific_fields: ProviderFields | None


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


class OriginalKeyPrecedence(TypedDict):
    operation: Literal["original_key_precedence"]
    prompt: str
    profiles: list[Profile]
    cases: list[MappingCase]


class SizeCases(TypedDict):
    operation: Literal["exact_vs_custom_ratio", "portrait_overlap"]
    profiles: list[Profile]
    sizes: list[JsonValue]


class UnknownSizeFallback(TypedDict):
    operation: Literal["unknown_size_fallback"]
    profiles: list[Profile]
    sizes: list[JsonValue]
    image_sizes: list[JsonValue]


class DevEndpoint(TypedDict):
    operation: Literal["dev_endpoint"]
    environments: list[dict[str, str]]
    api_bases: list[str | None]
    profiles: list[Profile]


class HiddenMetadataPresence(TypedDict):
    operation: Literal["hidden_metadata_presence"]
    profiles: list[Profile]
    replies: list[ReplyCase]


Input: TypeAlias = (
    OriginalKeyPrecedence
    | SizeCases
    | UnknownSizeFallback
    | DevEndpoint
    | HiddenMetadataPresence
)

EXISTING: TypeAdapter[list[ExistingImage]] = TypeAdapter(list[ExistingImage])
IMAGE_FIELDS: TypeAdapter[ImageFields] = TypeAdapter(ImageFields)


def config_for(profile: Profile) -> FluxConfig:
    if profile == "pro_v11":
        return FalAIFluxProV11Config()
    if profile == "schnell":
        return FalAIFluxSchnellConfig()
    if profile == "dev":
        return FalAIFluxDevConfig()
    return FalAIFluxProV11UltraConfig()


def apply_environment(environment: dict[str, str]) -> None:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


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


def precedence_outcome(config: FluxConfig, prompt: str, case: MappingCase) -> JsonValue:
    try:
        mapped = config.map_openai_params(
            dict(case["non_default"]), dict(case["optional"]), MODEL, case["drop_params"]
        )
        body = config.transform_image_generation_request(MODEL, prompt, mapped, {}, {})
    except ValueError:
        return {"error": {"status": None}}
    return {"request": body}


def original_key_precedence(call: OriginalKeyPrecedence) -> JsonValue:
    entries: list[JsonValue] = []
    for profile in call["profiles"]:
        config = config_for(profile)
        outcomes: list[JsonValue] = [
            precedence_outcome(config, call["prompt"], case) for case in call["cases"]
        ]
        supported = config.get_supported_openai_params(MODEL)
        entry: dict[str, JsonValue] = {
            "profile": profile,
            "supported": JSON.validate_python(supported),
            "cases": outcomes,
        }
        entries.append(entry)
    answer: dict[str, JsonValue] = {"profiles": entries}
    return JSON.validate_python(answer)


def mapping_outcome(config: FluxConfig, non_default: Params) -> JsonValue:
    try:
        mapped = config.map_openai_params(non_default, {}, MODEL, DROP_PARAMS)
    except ValueError:
        return {"error": {"status": None}}
    return {"params": mapped}


def size_cases(call: SizeCases | UnknownSizeFallback) -> JsonValue:
    cases: list[JsonValue] = []
    for profile in call["profiles"]:
        config = config_for(profile)
        sizes: list[JsonValue] = list(call["sizes"])
        if call["operation"] == "unknown_size_fallback" and profile != "ultra":
            sizes.extend(call["image_sizes"])
        for size in sizes:
            entry: dict[str, JsonValue] = {
                "profile": profile,
                "size": size,
                "outcome": mapping_outcome(config, {"size": size}),
            }
            cases.append(entry)
    answer: dict[str, JsonValue] = {"cases": cases}
    return JSON.validate_python(answer)


def dev_endpoint(call: DevEndpoint) -> JsonValue:
    entries: list[JsonValue] = []
    for environment in call["environments"]:
        apply_environment(environment)
        outcomes: list[JsonValue] = []
        for profile in call["profiles"]:
            config = config_for(profile)
            urls = [
                config.get_complete_url(api_base, None, MODEL, {}, {})
                for api_base in call["api_bases"]
            ]
            entry: dict[str, JsonValue] = {
                "profile": profile,
                "endpoint": config.IMAGE_GENERATION_ENDPOINT,
                "urls": JSON.validate_python(urls),
            }
            outcomes.append(entry)
        record: dict[str, JsonValue] = {
            "environment": JSON.validate_python(environment),
            "profiles": outcomes,
        }
        entries.append(record)
    answer: dict[str, JsonValue] = {"environments": entries}
    return JSON.validate_python(answer)


def image_record(image: Image) -> JsonValue:
    fields = IMAGE_FIELDS.validate_json(image.model_dump_json())
    return JSON.validate_python(fields)


def response_record(response: ImageResponse) -> JsonValue:
    data = response.data
    images: list[JsonValue] = []
    if data is not None:
        images = [image_record(image) for image in data]
    hidden = HiddenRecord.model_validate(response, from_attributes=True)
    answer: dict[str, JsonValue] = {"data": images, "hidden_params": hidden.hidden_params}
    return JSON.validate_python(answer)


def hidden_outcome(config: FluxConfig, reply: ReplyCase) -> JsonValue:
    model_response = ImageResponse(created=0)
    if reply["existing"]:
        model_response.data = [
            ImageObject(url=entry["url"], b64_json=entry["b64_json"])
            for entry in EXISTING.validate_python(reply["existing"])
        ]
    raw = httpx.Response(reply["status"], headers=reply["headers"], content=reply["body"])
    logging_obj = make_logging()
    try:
        result = config.transform_image_generation_response(
            MODEL, raw, model_response, logging_obj, {}, {}, {}, None
        )
    except BaseLLMException as error:
        raised = ErrorRecord.model_validate(error, from_attributes=True)
        return {"error": {"status": raised.status_code}}
    return response_record(result)


def hidden_metadata_presence(call: HiddenMetadataPresence) -> JsonValue:
    entries: list[JsonValue] = []
    for profile in call["profiles"]:
        config = config_for(profile)
        outcomes: list[JsonValue] = [
            hidden_outcome(config, reply) for reply in call["replies"]
        ]
        entry: dict[str, JsonValue] = {"profile": profile, "replies": outcomes}
        entries.append(entry)
    answer: dict[str, JsonValue] = {"profiles": entries}
    return JSON.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "original_key_precedence":
        return original_key_precedence(input_value)
    if input_value["operation"] == "dev_endpoint":
        return dev_endpoint(input_value)
    if input_value["operation"] == "hidden_metadata_presence":
        return hidden_metadata_presence(input_value)
    return size_cases(input_value)
