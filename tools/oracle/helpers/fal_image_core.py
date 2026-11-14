import os
from collections.abc import Callable
from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.llms.fal_ai.image_generation.gpt_image_2_transformation import (
    FalAIGPTImage2Config,
    map_gpt_image_quality,
    map_gpt_image_size,
    supported_gpt_image_qualities,
)
from litellm.llms.fal_ai.image_generation.transformation import (
    FalAIImageGenerationConfig,
    fal_images_to_image_objects,
)
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
HEADERS: TypeAdapter[dict[str, str]] = TypeAdapter(dict[str, str])
MODEL = "fal-ai/model"


class UrlConfig(Protocol):
    get_complete_url: Callable[..., str]


class MappingConfig(Protocol):
    map_openai_params: Callable[..., dict[str, JsonValue]]


class AuthConfig(Protocol):
    validate_environment: Callable[..., dict[str, str]]


class MappingCase(TypedDict):
    non_default: dict[str, JsonValue]
    optional: dict[str, JsonValue]
    drop_params: bool


class KeyCase(TypedDict):
    headers: dict[str, str]
    api_key: str | None


class ReplyCase(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class GenericBaseOnly(TypedDict):
    operation: Literal["generic_base_only"]
    environment: dict[str, str]
    api_bases: list[str | None]
    models: list[str]
    model: str
    cases: list[MappingCase]


class KeyScheme(TypedDict):
    operation: Literal["key_scheme"]
    environment: dict[str, str]
    cases: list[KeyCase]


class ImageReplyCases(TypedDict):
    operation: Literal["mixed_image_objects", "bool_dimensions_excluded"]
    replies: list[ReplyCase]


class GptDestinationPrecedence(TypedDict):
    operation: Literal["gpt_destination_precedence"]
    model: str
    cases: list[MappingCase]


class PricingQualityAlias(TypedDict):
    operation: Literal["pricing_quality_alias"]
    model: str
    catalog: dict[str, dict[str, JsonValue]]
    qualities: list[JsonValue]


class MalformedSizeKept(TypedDict):
    operation: Literal["malformed_size_kept"]
    sizes: list[JsonValue]


Input: TypeAlias = (
    GenericBaseOnly
    | KeyScheme
    | ImageReplyCases
    | GptDestinationPrecedence
    | PricingQualityAlias
    | MalformedSizeKept
)


class ProviderFields(TypedDict, total=False):
    width: int
    height: int
    content_type: str


class ImageFields(TypedDict, total=False):
    url: str | None
    b64_json: str | None
    provider_specific_fields: ProviderFields | None


def apply_environment(environment: dict[str, str]) -> None:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def mapping_outcome(config: MappingConfig, model: str, case: MappingCase) -> JsonValue:
    non_default = dict(case["non_default"])
    optional = dict(case["optional"])
    try:
        mapped = config.map_openai_params(non_default, optional, model, case["drop_params"])
    except ValueError:
        return JSON.validate_python({"error": {"status": None}})
    return JSON.validate_python({"params": PARAMS.validate_python(mapped)})


def url_pair(generic: UrlConfig, gpt: UrlConfig, api_base: str | None, model: str) -> JsonValue:
    return JSON.validate_python(
        {
            "generic": generic.get_complete_url(api_base, None, model, {}, {}),
            "gpt": gpt.get_complete_url(api_base, None, model, {}, {}),
        }
    )


def generic_base_only(call: GenericBaseOnly) -> JsonValue:
    apply_environment(call["environment"])
    generic = FalAIImageGenerationConfig()
    gpt = FalAIGPTImage2Config()
    urls: list[JsonValue] = [
        url_pair(generic, gpt, api_base, model)
        for api_base in call["api_bases"]
        for model in call["models"]
    ]
    return JSON.validate_python(
        {
            "urls": urls,
            "cases": [mapping_outcome(generic, call["model"], case) for case in call["cases"]],
        }
    )


def key_outcome(config: AuthConfig, case: KeyCase) -> JsonValue:
    try:
        headers = config.validate_environment(
            dict(case["headers"]), MODEL, [], {}, {}, api_key=case["api_key"], api_base=None
        )
    except ValueError:
        return JSON.validate_python({"error": {"status": None}})
    return JSON.validate_python({"headers": HEADERS.validate_python(headers)})


def key_scheme(call: KeyScheme) -> JsonValue:
    apply_environment(call["environment"])
    config = FalAIImageGenerationConfig()
    return JSON.validate_python(
        {"cases": [key_outcome(config, case) for case in call["cases"]]}
    )


def reply_outcome(case: ReplyCase) -> JsonValue:
    reply = PARAMS.validate_json(case["body"])
    images = fal_images_to_image_objects(reply.get("images"))
    return JSON.validate_python(
        {
            "images": [
                TypeAdapter(ImageFields).validate_json(image.model_dump_json())
                for image in images
            ]
        }
    )


def gpt_destination_precedence(call: GptDestinationPrecedence) -> JsonValue:
    config = FalAIGPTImage2Config()
    return JSON.validate_python(
        {"cases": [mapping_outcome(config, call["model"], case) for case in call["cases"]]}
    )


def pricing_quality_alias(call: PricingQualityAlias) -> JsonValue:
    supported = supported_gpt_image_qualities(call["model"], call["catalog"])
    mapped = [
        JSON.validate_python(map_gpt_image_quality(quality, call["model"], call["catalog"]))
        for quality in call["qualities"]
    ]
    return JSON.validate_python({"supported": sorted(supported), "mapped": mapped})


def malformed_size_kept(call: MalformedSizeKept) -> JsonValue:
    return JSON.validate_python(
        {"sizes": [JSON.validate_python(map_gpt_image_size(size)) for size in call["sizes"]]}
    )


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "generic_base_only":
        return generic_base_only(input_value)
    if input_value["operation"] == "key_scheme":
        return key_scheme(input_value)
    if input_value["operation"] == "gpt_destination_precedence":
        return gpt_destination_precedence(input_value)
    if input_value["operation"] == "pricing_quality_alias":
        return pricing_quality_alias(input_value)
    if input_value["operation"] == "malformed_size_kept":
        return malformed_size_kept(input_value)
    return JSON.validate_python(
        {"cases": [reply_outcome(case) for case in input_value["replies"]]}
    )
