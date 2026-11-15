import os
from collections.abc import Sequence
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.fal_ai.image_generation.bria_transformation import FalAIBriaConfig
from litellm.llms.fal_ai.image_generation.recraft_v3_transformation import (
    FalAIRecraftV3Config,
)
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

BRIA_MODEL = "bria/text-to-image/3.2"
RECRAFT_MODEL = "fal-ai/recraft/v3/text-to-image"
REQUEST_URL = "https://fal.run"

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class ProfileConfig(Protocol):
    def get_supported_openai_params(self, _model: str, /) -> Sequence[str]: ...

    def map_openai_params(
        self,
        _non_default_params: dict[str, JsonValue],
        _optional_params: dict[str, JsonValue],
        _model: str,
        /,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        _api_base: str | None,
        _api_key: str | None,
        _model: str,
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        /,
    ) -> str: ...

    def transform_image_generation_request(  # ast-grep-ignore: no-long-names
        self,
        _model: str,
        _prompt: str,
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def transform_image_generation_response(  # ast-grep-ignore: no-long-names
        self,
        _model: str,
        _raw_response: httpx.Response,
        _model_response: ImageResponse,
        _logging_obj: Logging,
        _request_data: dict[str, JsonValue],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _encoding: None,
        /,
    ) -> ImageResponse: ...


class MappingCase(TypedDict):
    non_default: dict[str, JsonValue]
    optional: dict[str, JsonValue]
    drop_params: bool


class BodyCase(TypedDict):
    prompt: str
    provider_options: dict[str, JsonValue]


class ReplyCase(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    existing: list[str]


class IgnoredCount(TypedDict):
    operation: Literal["ignored_count"]
    environment: dict[str, str]
    api_bases: list[str | None]
    cases: list[MappingCase]
    bodies: list[BodyCase]


class BriaOverlapEdges(TypedDict):
    operation: Literal["bria_overlap_edges"]
    sizes: list[str]


class RecraftPresetDimensions(TypedDict):
    operation: Literal["recraft_preset_dimensions"]
    sizes: list[str]


class SingularVsPlural(TypedDict):
    operation: Literal["singular_vs_plural"]
    bria: list[ReplyCase]
    recraft: list[ReplyCase]


class EmptyImageMap(TypedDict):
    operation: Literal["empty_image_map"]
    bria: list[ReplyCase]
    recraft: list[ReplyCase]


class DiscardedReplyMetadata(TypedDict):
    operation: Literal["discarded_reply_metadata"]
    bria: list[ReplyCase]
    recraft: list[ReplyCase]


SizeCall: TypeAlias = BriaOverlapEdges | RecraftPresetDimensions
ReplyCall: TypeAlias = SingularVsPlural | EmptyImageMap | DiscardedReplyMetadata
Input: TypeAlias = IgnoredCount | SizeCall | ReplyCall


class ImageUrl(TypedDict):
    url: str | None
    b64_json: str | None
    revised_prompt: str | None


class ImageResponseFields(TypedDict):
    created: int
    data: list[ImageUrl]


class ExistingImage(TypedDict):
    url: str


class ReplyPayload(TypedDict):
    created: int
    data: list[ExistingImage]


RESPONSE: TypeAdapter[ImageResponse] = TypeAdapter(ImageResponse)
IMAGES: TypeAdapter[ImageResponseFields] = TypeAdapter(ImageResponseFields)
# LiteLLM's class carries Any in its constructor, so the catch names it through a typed alias
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


class Recorder(Logging):
    def __init__(self) -> None:
        self.model_call_details: dict[str, JsonValue] = {}


def apply_environment(environment: dict[str, str]) -> None:
    for name in ("FAL_AI_API_KEY", "FAL_AI_API_BASE"):
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def mapping_outcome(config: ProfileConfig, model: str, case: MappingCase) -> JsonValue:
    try:
        mapped = config.map_openai_params(
            dict(case["non_default"]),
            dict(case["optional"]),
            model,
            drop_params=case["drop_params"],
        )
    except ValueError:
        return {"error": {"status": None}}
    return {"params": PARAMS.validate_python(mapped)}


def body_outcome(config: ProfileConfig, model: str, case: BodyCase) -> JsonValue:
    body = config.transform_image_generation_request(
        model, case["prompt"], dict(case["provider_options"]), {}, {}
    )
    return JSON.validate_python(body)


def model_report(config: ProfileConfig, model: str, call: IgnoredCount) -> JsonValue:
    return {
        "supported": JSON.validate_python(config.get_supported_openai_params(model)),
        "urls": [
            config.get_complete_url(api_base, None, model, {}, {})
            for api_base in call["api_bases"]
        ],
        "cases": [mapping_outcome(config, model, case) for case in call["cases"]],
        "bodies": [body_outcome(config, model, case) for case in call["bodies"]],
    }


def ignored_count(call: IgnoredCount) -> JsonValue:
    apply_environment(call["environment"])
    return {
        "bria": model_report(FalAIBriaConfig(), BRIA_MODEL, call),
        "recraft": model_report(FalAIRecraftV3Config(), RECRAFT_MODEL, call),
    }


def size_list(config: ProfileConfig, model: str, call: SizeCall) -> JsonValue:
    outcomes: list[JsonValue] = []
    for size in call["sizes"]:
        case = MappingCase(non_default={"size": size}, optional={}, drop_params=False)
        outcomes.append(mapping_outcome(config, model, case))
    return {"sizes": outcomes}


def reply_outcome(config: ProfileConfig, case: ReplyCase) -> JsonValue:
    payload: ReplyPayload = {
        "created": 0,
        "data": [ExistingImage(url=url) for url in case["existing"]],
    }
    request = httpx.Request("POST", REQUEST_URL)
    raw = httpx.Response(
        status_code=case["status"],
        content=case["body"].encode(),
        request=request,
    )
    try:
        result = config.transform_image_generation_response(
            "",
            raw,
            RESPONSE.validate_python(payload),
            Recorder(),
            {},
            {},
            {},
            None,
        )
    except (ValueError, PROVIDER_ERROR):
        return {"error": {"status": case["status"]}}
    dumped = IMAGES.validate_json(result.model_dump_json())
    return {"images": JSON.validate_python(dumped["data"])}


def reply_pair(
    bria_config: ProfileConfig, recraft_config: ProfileConfig, call: ReplyCall
) -> JsonValue:
    return {
        "bria": [reply_outcome(bria_config, case) for case in call["bria"]],
        "recraft": [
            reply_outcome(recraft_config, case) for case in call["recraft"]
        ],
    }


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "ignored_count":
        return ignored_count(input_value)
    if input_value["operation"] == "bria_overlap_edges":
        return size_list(FalAIBriaConfig(), BRIA_MODEL, input_value)
    if input_value["operation"] == "recraft_preset_dimensions":
        return size_list(FalAIRecraftV3Config(), RECRAFT_MODEL, input_value)
    return reply_pair(FalAIBriaConfig(), FalAIRecraftV3Config(), input_value)
