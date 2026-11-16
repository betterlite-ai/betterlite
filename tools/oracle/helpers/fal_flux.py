import os
from collections.abc import Callable, Sequence
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, TypeVar

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.fal_ai.image_generation.flux_dev_transformation import FalAIFluxDevConfig
from litellm.llms.fal_ai.image_generation.flux_pro_v11_transformation import FalAIFluxProV11Config
from litellm.llms.fal_ai.image_generation.flux_pro_v11_ultra_transformation import (
    FalAIFluxProV11UltraConfig,
)
from litellm.llms.fal_ai.image_generation.flux_schnell_transformation import FalAIFluxSchnellConfig
from litellm.types.utils import ImageObject, ImageResponse
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MODEL = "fal-ai/model"
Profile: TypeAlias = Literal["ultra", "pro_v11", "schnell", "dev"]
Params: TypeAlias = dict[str, JsonValue]
Payload = TypeVar("Payload")
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


GenerationRequest: TypeAlias = Callable[[str, str, Params, Params, dict[str, str]], Params]
GenerationResponse: TypeAlias = Callable[
    [str, httpx.Response, ImageResponse, Logging, Params, Params, Params, None], ImageResponse
]


class FluxConfig(Protocol):
    IMAGE_GENERATION_ENDPOINT: str
    @property
    def get_supported_openai_params(self) -> Callable[[str], Sequence[str]]: ...
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
CONFIGS: Final[dict[Profile, FluxConfig]] = {
    "ultra": FalAIFluxProV11UltraConfig(),
    "pro_v11": FalAIFluxProV11Config(),
    "schnell": FalAIFluxSchnellConfig(),
    "dev": FalAIFluxDevConfig(),
}


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


def profile_entries(
    profiles: list[Profile],
    call: Payload,
    entries_for: Callable[[Profile, FluxConfig, Payload], dict[str, JsonValue]],
) -> list[JsonValue]:
    entries: list[JsonValue] = [
        entries_for(profile, CONFIGS[profile], call) for profile in profiles
    ]
    return entries


def precedence_outcome(config: FluxConfig, prompt: str, case: MappingCase) -> JsonValue:
    try:
        mapped = config.map_openai_params(
            dict(case["non_default"]), dict(case["optional"]), MODEL, case["drop_params"]
        )
        body = config.transform_image_generation_request(MODEL, prompt, mapped, {}, {})
    except ValueError:
        return {"error": {"status": None}}
    return {"request": body}


def precedence_entry(
    profile: Profile, config: FluxConfig, call: OriginalKeyPrecedence
) -> dict[str, JsonValue]:
    supported = config.get_supported_openai_params(MODEL)
    outcomes = [precedence_outcome(config, call["prompt"], case) for case in call["cases"]]
    return {
        "profile": profile,
        "supported": JSON.validate_python(supported),
        "cases": outcomes,
    }


def original_key_precedence(call: OriginalKeyPrecedence) -> JsonValue:
    entries = profile_entries(call["profiles"], call, precedence_entry)
    return JSON.validate_python({"profiles": entries})


def mapping_outcome(config: FluxConfig, non_default: Params) -> JsonValue:
    try:
        mapped = config.map_openai_params(non_default, {}, MODEL, False)
    except ValueError:
        return {"error": {"status": None}}
    return {"params": mapped}


def size_cases(call: SizeCases | UnknownSizeFallback) -> JsonValue:
    cases: list[JsonValue] = []
    for profile in call["profiles"]:
        config = CONFIGS[profile]
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
    except PROVIDER_ERROR as error:
        raised = ErrorRecord.model_validate(error, from_attributes=True)
        return {"error": {"status": raised.status_code}}
    data = result.data
    images: list[JsonValue] = []
    if data is not None:
        images = [
            JSON.validate_python(IMAGE_FIELDS.validate_json(image.model_dump_json()))
            for image in data
        ]
    hidden = HiddenRecord.model_validate(result, from_attributes=True)
    answer: dict[str, JsonValue] = {"data": images, "hidden_params": hidden.hidden_params}
    return JSON.validate_python(answer)


def replies_entry(
    profile: Profile, config: FluxConfig, call: HiddenMetadataPresence
) -> dict[str, JsonValue]:
    outcomes = [hidden_outcome(config, reply) for reply in call["replies"]]
    return {"profile": profile, "replies": outcomes}


def hidden_metadata_presence(call: HiddenMetadataPresence) -> JsonValue:
    entries = profile_entries(call["profiles"], call, replies_entry)
    return JSON.validate_python({"profiles": entries})


def endpoint_entry(
    profile: Profile, config: FluxConfig, call: DevEndpoint
) -> dict[str, JsonValue]:
    urls = [config.get_complete_url(base, None, MODEL, {}, {}) for base in call["api_bases"]]
    return {
        "profile": profile,
        "endpoint": config.IMAGE_GENERATION_ENDPOINT,
        "urls": JSON.validate_python(urls),
    }


def dev_endpoint(call: DevEndpoint) -> JsonValue:
    environments: list[JsonValue] = []
    for environment in call["environments"]:
        apply_environment(environment)
        profiles = profile_entries(call["profiles"], call, endpoint_entry)
        record: dict[str, JsonValue] = {
            "environment": JSON.validate_python(environment),
            "profiles": profiles,
        }
        environments.append(record)
    return JSON.validate_python({"environments": environments})


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "original_key_precedence":
        return original_key_precedence(input_value)
    if input_value["operation"] == "dev_endpoint":
        return dev_endpoint(input_value)
    if input_value["operation"] == "hidden_metadata_presence":
        return hidden_metadata_presence(input_value)
    return size_cases(input_value)
