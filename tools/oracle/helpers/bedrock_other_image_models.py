import json
from collections.abc import Callable
from enum import StrEnum
from typing import Protocol, TypeAlias, cast

from litellm.llms.bedrock.image_generation.amazon_nova_canvas_transformation import (
    AmazonNovaCanvasConfig,
)
from litellm.llms.bedrock.image_generation.amazon_stability1_transformation import (
    AmazonStabilityConfig,
)
from litellm.llms.bedrock.image_generation.amazon_stability3_transformation import (
    AmazonStability3Config,
)
from litellm.llms.bedrock.image_generation.amazon_titan_transformation import (
    AmazonTitanImageGenerationConfig,
)
from litellm.llms.bedrock.image_generation.image_handler import BedrockImageGeneration
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
ConfigClass: TypeAlias = (
    type[AmazonTitanImageGenerationConfig]
    | type[AmazonStabilityConfig]
    | type[AmazonStability3Config]
    | type[AmazonNovaCanvasConfig]
)


class FamilyName(StrEnum):
    titan = "titan"
    stability1 = "stability1"
    stability3 = "stability3"
    nova = "nova"


CONFIG_CLASSES: dict[FamilyName, ConfigClass] = {
    FamilyName.titan: AmazonTitanImageGenerationConfig,
    FamilyName.stability1: AmazonStabilityConfig,
    FamilyName.stability3: AmazonStability3Config,
    FamilyName.nova: AmazonNovaCanvasConfig,
}
FAMILY_NAMES: dict[ConfigClass, FamilyName] = {value: key for key, value in CONFIG_CLASSES.items()}


class ImageConfig(Protocol):
    def map_openai_params(
        self, non_default_params: dict[str, Json], optional_params: dict[str, Json]
    ) -> dict[str, Json]: ...
    def transform_request_body(
        self, text: str, optional_params: dict[str, Json]
    ) -> dict[str, Json]: ...
    def transform_response_dict_to_openai_response(
        self, model_response: ImageResponse, response_dict: dict[str, Json]
    ) -> ImageResponse: ...


class TitanCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    non_default: dict[str, Json] = {}
    optional_params: dict[str, Json] = {}
    text: str = ""


class SettingsCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    cfg_scale: Json = None
    seed: Json = None
    steps: Json = None
    width: Json = None
    height: Json = None


class Stability1Call(BaseModel):
    model_config = ConfigDict(extra="forbid")
    non_default: dict[str, Json] = {}
    optional_params: dict[str, Json] = {}
    text: str = ""
    settings: SettingsCall = SettingsCall()


class TextParamsCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    text: str = ""
    optional_params: dict[str, Json] = {}


class SizeQualityCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    family: FamilyName | None = None
    non_default: dict[str, Json] = {}
    optional_params: dict[str, Json] = {}


class ArtifactsCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    family: FamilyName | None = None
    response_body: dict[str, Json] = {}


class ProviderErrorCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    family: FamilyName | None = None
    response_body: dict[str, Json] | None = None
    optional_params: dict[str, Json] = {}
    text: str = ""


class SelectionCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str | None = None


def to_json(value: object) -> Json:
    return cast("Json", json.loads(json.dumps(value)))


def family_or_stability1(name: FamilyName | None) -> FamilyName:
    return name or FamilyName.stability1


def config_class(name: FamilyName | None) -> ConfigClass:
    return CONFIG_CLASSES[family_or_stability1(name)]


def as_image_config(cls: ConfigClass) -> ImageConfig:
    return cast(ImageConfig, cls())


def images_from(model_response: ImageResponse) -> Json:
    return {
        "created": model_response.created,
        "data": [{"b64_json": image.b64_json} for image in model_response.data or []],
    }


def titan_image_body(argument: Json) -> Json:
    call = TitanCall.model_validate(argument)
    config = as_image_config(AmazonTitanImageGenerationConfig)
    mapped = config.map_openai_params(dict(call.non_default), dict(call.optional_params))
    body = config.transform_request_body(call.text, mapped)
    return to_json(body)


def stability1_text_prompts(argument: Json) -> Json:
    call = Stability1Call.model_validate(argument)
    settings = call.settings
    AmazonStabilityConfig(
        cfg_scale=cast("int | None", settings.cfg_scale),
        seed=cast("float | None", settings.seed),
        steps=cast("list[str] | None", settings.steps),
        width=cast("int | None", settings.width),
        height=cast("int | None", settings.height),
    )
    config = as_image_config(AmazonStabilityConfig)
    mapped = config.map_openai_params(dict(call.non_default), dict(call.optional_params))
    body = config.transform_request_body(call.text, mapped)
    return to_json(body)


def stability3_prompt(argument: Json) -> Json:
    call = TextParamsCall.model_validate(argument)
    config = as_image_config(AmazonStability3Config)
    mapped = config.map_openai_params({}, dict(call.optional_params))
    body = config.transform_request_body(call.text, mapped)
    return to_json(body)


def nova_canvas_task_body(argument: Json) -> Json:
    call = TextParamsCall.model_validate(argument)
    config = as_image_config(AmazonNovaCanvasConfig)
    body = config.transform_request_body(call.text, dict(call.optional_params))
    return to_json(body)


def size_n_quality_map(argument: Json) -> Json:
    call = SizeQualityCall.model_validate(argument)
    config = as_image_config(config_class(call.family))
    mapped = config.map_openai_params(dict(call.non_default), dict(call.optional_params))
    return to_json(mapped)


def artifacts_to_b64_results(argument: Json) -> Json:
    call = ArtifactsCall.model_validate(argument)
    config = as_image_config(config_class(call.family))
    model_response = config.transform_response_dict_to_openai_response(
        ImageResponse(created=42), dict(call.response_body)
    )
    return images_from(model_response)


def image_provider_error(argument: Json) -> Json:
    call = ProviderErrorCall.model_validate(argument)
    config = as_image_config(config_class(call.family))
    if call.response_body is not None:
        model_response = config.transform_response_dict_to_openai_response(
            ImageResponse(created=42), dict(call.response_body)
        )
        return images_from(model_response)
    body = config.transform_request_body(call.text, dict(call.optional_params))
    return to_json(body)


def image_family_selection(argument: Json) -> Json:
    call = SelectionCall.model_validate(argument)
    cls = BedrockImageGeneration.get_config_class(call.model)
    return FAMILY_NAMES[cls].value


HANDLERS: dict[str, Callable[[Json], Json]] = {
    "titan_image_body": titan_image_body,
    "stability1_text_prompts": stability1_text_prompts,
    "stability3_prompt": stability3_prompt,
    "nova_canvas_task_body": nova_canvas_task_body,
    "size_n_quality_map": size_n_quality_map,
    "artifacts_to_b64_results": artifacts_to_b64_results,
    "image_provider_error": image_provider_error,
    "image_family_selection": image_family_selection,
}


def run(input_value: dict[str, Json]) -> Json:
    name, argument = next(iter(input_value.items()))
    return HANDLERS[name](argument)
