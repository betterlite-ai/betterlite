import os
from collections.abc import Callable
from typing import Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.fal_ai.image_generation.ideogram_v3_transformation import (
    FalAIIdeogramV3Config,
)
from litellm.llms.fal_ai.image_generation.stable_diffusion_transformation import (
    FalAIStableDiffusionConfig,
)
from litellm.types.utils import ImageObject, ImageResponse
from openai.types.image import Image
from pydantic import BaseModel, Field, JsonValue, TypeAdapter, ValidationError

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
IDEOGRAM_MODEL = "fal-ai/ideogram/v3"
DIFFUSION_MODEL = "stable-diffusion-v35-medium"
REPLY_MODEL = "fal-ai/model"
PROMPT = "a prompt"


class Recorder(Logging):
    def __init__(self) -> None:
        pass


class RenderConfig(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def transform_image_generation_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        prompt: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
    ) -> dict[str, JsonValue]: ...


# LiteLLM types the request dicts bare, so this alias names what the call passes
ReplyTransform: TypeAlias = Callable[
    [
        str,
        httpx.Response,
        ImageResponse,
        Recorder,
        dict[str, JsonValue],
        dict[str, JsonValue],
        dict[str, JsonValue],
        None,
    ],
    ImageResponse,
]


class ReplyConfig(Protocol):
    @property
    def transform_image_generation_response(  # ast-grep-ignore: no-long-names
        self,
    ) -> ReplyTransform: ...


class HiddenRecord(BaseModel):
    hidden_params: dict[str, JsonValue] = Field(alias="_hidden_params")


class MappingCase(TypedDict):
    non_default: dict[str, JsonValue]
    optional: dict[str, JsonValue]
    drop_params: bool


class ImageSeed(TypedDict):
    b64_json: str | None
    url: str | None
    revised_prompt: str | None


class ReplyCase(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    data: list[ImageSeed]
    hidden_params: dict[str, JsonValue]


class SizeCases(TypedDict):
    operation: Literal["whitespace_size", "size_object_extra_fields", "portrait_preset_difference"]
    sizes: list[JsonValue]


class CustomDiffusionPathInput(TypedDict):
    operation: Literal["custom_diffusion_path"]
    environment: dict[str, str]
    api_bases: list[str | None]
    models: list[str]
    model: str
    prompt: str
    cases: list[MappingCase]


class ReplyCases(TypedDict):
    operation: Literal["invalid_reply_member", "hidden_fields_by_family"]
    replies: list[ReplyCase]


Input: TypeAlias = SizeCases | CustomDiffusionPathInput | ReplyCases

IDEOGRAM = FalAIIdeogramV3Config()
DIFFUSION = FalAIStableDiffusionConfig()
# the alias types the class itself, since the class object's constructor takes bare dicts
REPLY_ERROR: type[BaseLLMException] = BaseLLMException


def apply_environment(environment: dict[str, str]) -> None:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def mapping_entry(config: RenderConfig, model: str, prompt: str, case: MappingCase) -> JsonValue:
    try:
        mapped = config.map_openai_params(
            dict(case["non_default"]),
            dict(case["optional"]),
            model,
            drop_params=case["drop_params"],
        )
    except (TypeError, ValueError):
        return {"error": {"status": None}}
    body = config.transform_image_generation_request(model, prompt, mapped, {}, {})
    answer: dict[str, JsonValue] = {"params": PARAMS.validate_python(mapped), "body": body}
    return JSON.validate_python(answer)


def size_entry(size: JsonValue) -> JsonValue:
    case: MappingCase = {"non_default": {"size": size}, "optional": {}, "drop_params": False}
    answer: dict[str, JsonValue] = {
        "ideogram": mapping_entry(IDEOGRAM, IDEOGRAM_MODEL, PROMPT, case),
        "stable_diffusion": mapping_entry(DIFFUSION, DIFFUSION_MODEL, PROMPT, case),
    }
    return JSON.validate_python(answer)


def size_outcomes(call: SizeCases) -> JsonValue:
    sizes = [size_entry(size) for size in call["sizes"]]
    return {"sizes": sizes}


def case_entry(call: CustomDiffusionPathInput, case: MappingCase) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "ideogram": mapping_entry(IDEOGRAM, call["model"], call["prompt"], case),
        "stable_diffusion": mapping_entry(DIFFUSION, call["model"], call["prompt"], case),
    }
    return JSON.validate_python(answer)


def custom_diffusion_path(call: CustomDiffusionPathInput) -> JsonValue:
    apply_environment(call["environment"])
    optional_params: dict[str, JsonValue] = {}
    litellm_params: dict[str, JsonValue] = {}
    urls: list[JsonValue] = []
    for api_base in call["api_bases"]:
        for model in call["models"]:
            pair: dict[str, JsonValue] = {
                "ideogram": IDEOGRAM.get_complete_url(
                    api_base, None, model, optional_params, litellm_params
                ),
                "stable_diffusion": DIFFUSION.get_complete_url(
                    api_base, None, model, optional_params, litellm_params
                ),
            }
            urls.append(JSON.validate_python(pair))
    cases = [case_entry(call, case) for case in call["cases"]]
    return {"urls": urls, "cases": cases}


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


def images_value(response: ImageResponse) -> JsonValue:
    data = response.data
    if data is None:
        return []
    entries: list[JsonValue] = []
    for image in data:
        entry: ImageSeed = {
            "b64_json": image.b64_json,
            "url": image.url,
            "revised_prompt": image.revised_prompt,
        }
        entries.append(JSON.validate_python(entry))
    return JSON.validate_python(entries)


def hidden_value(response: ImageResponse) -> JsonValue:
    record = HiddenRecord.model_validate(response, from_attributes=True)
    return JSON.validate_python(record.hidden_params)


def reply_entry(config: ReplyConfig, case: ReplyCase) -> JsonValue:
    seeded: dict[str, JsonValue] = dict(case["hidden_params"])
    model_response = ImageResponse(created=1, hidden_params=seeded)
    if case["data"]:
        model_response.data = seeded_images(case["data"])
    raw_response = httpx.Response(
        case["status"], headers=dict(case["headers"]), content=case["body"]
    )
    request_data: dict[str, JsonValue] = {}
    optional_params: dict[str, JsonValue] = {}
    litellm_params: dict[str, JsonValue] = {}
    try:
        finished = config.transform_image_generation_response(
            REPLY_MODEL,
            raw_response,
            model_response,
            Recorder(),
            request_data,
            optional_params,
            litellm_params,
            None,
        )
    except REPLY_ERROR as error:
        return {"error": {"status": error.status_code}}
    except ValidationError:
        return {"error": {"status": None}}
    answer: dict[str, JsonValue] = {
        "data": images_value(finished),
        "hidden_params": hidden_value(finished),
    }
    return JSON.validate_python(answer)


def reply_pair(case: ReplyCase) -> JsonValue:
    answer: dict[str, JsonValue] = {
        "ideogram": reply_entry(IDEOGRAM, case),
        "stable_diffusion": reply_entry(DIFFUSION, case),
    }
    return JSON.validate_python(answer)


def reply_cases(call: ReplyCases) -> JsonValue:
    cases = [reply_pair(case) for case in call["replies"]]
    return {"cases": cases}


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "whitespace_size" | "size_object_extra_fields" | "portrait_preset_difference":
            return size_outcomes(input_value)
        case "custom_diffusion_path":
            return custom_diffusion_path(input_value)
        case "invalid_reply_member" | "hidden_fields_by_family":
            return reply_cases(input_value)
