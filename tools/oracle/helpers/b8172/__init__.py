import os
from typing import Final

import httpx
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.dashscope.common_utils import (
    get_dashscope_family_embedding_config,
    get_dashscope_family_image_generation_config,
    get_dashscope_family_rerank_config,
    missing_dashscope_family_key_message,
    resolve_dashscope_family_api_key,
)
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

from .views import (
    EmbeddingBoundary,
    EmbeddingUrlConfig,
    FamilyKeyAndMessageInput,
    FamilySettingsInput,
    FamilyStep,
    ImageBoundary,
    ImageConfig,
    ImageParamsAndSizeInput,
    ImageReplyDump,
    ImageReplyInput,
    ImageRequestInput,
    ImageUrlAndKeyInput,
    Input,
    NoMessages,
    Recorder,
    RerankBoundary,
    RerankUrlConfig,
)

EMBEDDING_MODEL = "text-embedding-v4"
IMAGE_MODEL = "qwen-image-2.0-pro"
KEEP_PARAMS: Final = False
PROBE_MODEL = "/probe"
RERANK_MODEL = "qwen3-rerank"


def image_config(family: str) -> ImageConfig:
    values = {"config": get_dashscope_family_image_generation_config(family)}
    return ImageBoundary.model_validate(values).config


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "image_params_and_size":
        return image_params_and_size(input_value)
    if input_value["function"] == "image_url_and_key":
        return image_url_and_key(input_value)
    if input_value["function"] == "image_request":
        return image_request(input_value)
    if input_value["function"] == "image_reply":
        return image_reply(input_value)
    if input_value["function"] == "family_settings":
        return family_settings(input_value)
    return family_key_and_message(input_value)


def image_params_and_size(input_value: ImageParamsAndSizeInput) -> JsonValue:
    mapped = image_config("dashscope").map_openai_params(
        input_value["non_default_params"],
        input_value["optional_params"],
        IMAGE_MODEL,
        drop_params=KEEP_PARAMS,
    )
    return {"params": mapped}


def image_url_and_key(input_value: ImageUrlAndKeyInput) -> JsonValue:
    config = image_config("dashscope")
    url = config.get_complete_url(input_value.get("api_base"), None, IMAGE_MODEL, {}, {})
    caller_headers = input_value.get("caller_headers")
    headers = config.validate_environment(
        {} if caller_headers is None else dict(caller_headers),
        IMAGE_MODEL,
        NoMessages(),
        {},
        {},
        input_value.get("api_key"),
    )
    return {"url": url, "headers": headers}


def image_request(input_value: ImageRequestInput) -> JsonValue:
    body = image_config("dashscope").transform_image_generation_request(
        input_value["model"], input_value["prompt"], input_value["optional_params"], {}, {}
    )
    return {"body": body}


def image_reply(input_value: ImageReplyInput) -> JsonValue:
    config = image_config("dashscope")
    headers = input_value.get("headers")
    raw_response = httpx.Response(
        input_value["status"],
        content=input_value["body"].encode(),
        headers={} if headers is None else headers,
    )
    result = config.transform_image_generation_response(
        IMAGE_MODEL, raw_response, ImageResponse(), Recorder(), {}, {}, {}, None, None, None
    )
    dumped = TypeAdapter(ImageReplyDump).validate_json(result.model_dump_json())
    images: list[JsonValue] = []
    for item in dumped["data"]:
        image: dict[str, JsonValue] = {"url": item["url"]}
        images.append(image)
    return {"data": images}


def set_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def apply_env(family: str, step: FamilyStep) -> None:
    prefix = "DASHSCOPE" if family == "dashscope" else family.upper()
    set_env(f"{prefix}_API_BASE", step.get("base"))
    set_env("DASHSCOPE_API_KEY", step.get("fallback_key"))
    set_env(f"{prefix}_API_BASE_RERANK", step.get("rerank_base"))
    set_env(f"{prefix}_API_BASE_IMAGE", step.get("image_base"))
    if prefix != "DASHSCOPE":
        set_env(f"{prefix}_API_KEY", step.get("brand_key"))


def family_value(
    family: str, step: FamilyStep, api_base: str | None, api_key: str | None
) -> JsonValue:
    apply_env(family, step)
    image = image_config(family)
    embedding_values = {"config": get_dashscope_family_embedding_config(family)}
    embedding: EmbeddingUrlConfig = EmbeddingBoundary.model_validate(embedding_values).config
    rerank_values = {"config": get_dashscope_family_rerank_config(family)}
    rerank: RerankUrlConfig = RerankBoundary.model_validate(rerank_values).config
    _model, _provider, key, base = get_llm_provider(
        model=f"{family}{PROBE_MODEL}", api_base=api_base, api_key=api_key
    )
    return {
        "chat": {"api_base": base, "api_key": key},
        "embedding_url": embedding.get_complete_url(api_base, None, EMBEDDING_MODEL, {}, {}),
        "rerank_url": rerank.get_complete_url(api_base, RERANK_MODEL, None),
        "image": {
            "url": image.get_complete_url(api_base, None, IMAGE_MODEL, {}, {}),
            "headers": image.validate_environment({}, IMAGE_MODEL, NoMessages(), {}, {}, None),
        },
    }


def family_settings(input_value: FamilySettingsInput) -> JsonValue:
    api_base = input_value.get("api_base")
    api_key = input_value.get("api_key")
    return {
        "dashscope": family_value("dashscope", input_value["dashscope"], api_base, api_key),
        "qwencloud": family_value("qwencloud", input_value["qwencloud"], api_base, api_key),
        "qwen_ai_platform": family_value(
            "qwen_ai_platform", input_value["qwen_ai_platform"], api_base, api_key
        ),
    }


def family_key_and_message(input_value: FamilyKeyAndMessageInput) -> JsonValue:
    api_key = input_value.get("api_key")
    set_env("DASHSCOPE_API_KEY", input_value.get("dashscope"))
    set_env("QWENCLOUD_API_KEY", input_value.get("qwencloud"))
    set_env("QWEN_AI_PLATFORM_API_KEY", input_value.get("qwen_ai_platform"))
    result: dict[str, JsonValue] = {}
    for family in ("dashscope", "qwencloud", "qwen_ai_platform"):
        key = resolve_dashscope_family_api_key(family, api_key)
        result[family] = {"key": key, "message": missing_dashscope_family_key_message(family)}
    return result
