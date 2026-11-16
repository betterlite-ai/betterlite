import os
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.dashscope.common_utils import (
    get_dashscope_family_embedding_config,
    get_dashscope_family_image_generation_config,
    get_dashscope_family_rerank_config,
    missing_dashscope_family_key_message,
    resolve_dashscope_family_api_key,
)
from litellm.llms.dashscope.image_generation.transformation import DashScopeImageGenerationConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

IMAGE_MODEL = "qwen-image-2.0-pro"
EMBEDDING_MODEL = "text-embedding-v4"
RERANK_MODEL = "qwen3-rerank"
PROBE_MODEL = "/probe"
KEEP_PARAMS: Final = False


class NoMessages(list[AllMessageValues]): ...


class Recorder(Logging):
    def __init__(self) -> None:
        pass

    @override
    def post_call(
        self,
        original_response: JsonValue,
        input: JsonValue = None,
        api_key: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        del original_response, input, api_key, additional_args


class ImageConfig(Protocol):
    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
        /,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: NoMessages,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        api_key: str | None,
        /,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        /,
    ) -> str: ...

    def transform_image_generation_request(
        self,
        model: str,
        prompt: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        headers: dict[str, str],
        /,
    ) -> dict[str, JsonValue]: ...

    def transform_image_generation_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ImageResponse,
        logging_obj: Recorder,
        request_data: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        encoding: None,
        api_key: None,
        json_mode: None,
        /,
    ) -> ImageResponse: ...


class EmbeddingUrlConfig(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        /,
    ) -> str: ...


class RerankUrlConfig(Protocol):
    def get_complete_url(
        self,
        api_base: str | None,
        model: str,
        optional_params: dict[str, JsonValue] | None,
        /,
    ) -> str: ...


class ImageParamsAndSizeInput(TypedDict):
    function: Literal["image_params_and_size"]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class ImageUrlAndKeyInput(TypedDict):
    function: Literal["image_url_and_key"]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    env_key: NotRequired[str | None]
    env_image_base: NotRequired[str | None]
    caller_headers: NotRequired[dict[str, str]]


class ImageRequestInput(TypedDict):
    function: Literal["image_request"]
    model: str
    prompt: str
    optional_params: dict[str, JsonValue]


class ImageReplyInput(TypedDict):
    function: Literal["image_reply"]
    body: str
    status: int
    headers: NotRequired[dict[str, str]]


class FamilyStep(TypedDict):
    base: NotRequired[str | None]
    brand_key: NotRequired[str | None]
    fallback_key: NotRequired[str | None]
    rerank_base: NotRequired[str | None]
    image_base: NotRequired[str | None]


class FamilySettingsInput(TypedDict):
    function: Literal["family_settings"]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    dashscope: FamilyStep
    qwencloud: FamilyStep
    qwen_ai_platform: FamilyStep


class FamilyKeyAndMessageInput(TypedDict):
    function: Literal["family_key_and_message"]
    api_key: NotRequired[str | None]
    dashscope_key: NotRequired[str | None]
    qwencloud_key: NotRequired[str | None]
    qwen_ai_platform_key: NotRequired[str | None]


class ImageDatum(TypedDict):
    url: str | None


class ImageReplyDump(TypedDict):
    data: list[ImageDatum]


class EnvNames(TypedDict):
    base: str
    brand_key: str
    fallback_key: str
    rerank_base: str
    image_base: str


Input = (
    ImageParamsAndSizeInput
    | ImageUrlAndKeyInput
    | ImageRequestInput
    | ImageReplyInput
    | FamilySettingsInput
    | FamilyKeyAndMessageInput
)


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
    config: ImageConfig = DashScopeImageGenerationConfig()
    mapped = config.map_openai_params(
        input_value["non_default_params"],
        input_value["optional_params"],
        IMAGE_MODEL,
        KEEP_PARAMS,
    )
    return {"params": mapped}


def image_url_and_key(input_value: ImageUrlAndKeyInput) -> JsonValue:
    config: ImageConfig = DashScopeImageGenerationConfig()
    url = config.get_complete_url(
        input_value.get("api_base"), None, IMAGE_MODEL, {}, {}
    )
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
    config: ImageConfig = DashScopeImageGenerationConfig()
    body = config.transform_image_generation_request(
        input_value["model"],
        input_value["prompt"],
        input_value["optional_params"],
        {},
        {},
    )
    return {"body": body}


def image_reply(input_value: ImageReplyInput) -> JsonValue:
    config: ImageConfig = get_dashscope_family_image_generation_config("dashscope")
    headers = input_value.get("headers")
    raw_response = httpx.Response(
        input_value["status"],
        content=input_value["body"].encode(),
        headers={} if headers is None else headers,
    )
    result = config.transform_image_generation_response(
        IMAGE_MODEL,
        raw_response,
        ImageResponse(),
        Recorder(),
        {},
        {},
        {},
        None,
        None,
        None,
    )
    dumped = TypeAdapter(ImageReplyDump).validate_json(result.model_dump_json())
    return {"data": dumped["data"]}


def set_env(name: str, value: str | None) -> None:
    if value is None:
        if name in os.environ:
            del os.environ[name]
    else:
        os.environ[name] = value


def env_names(family: str) -> EnvNames:
    if family == "qwencloud":
        return EnvNames(
            base="QWENCLOUD_API_BASE",
            brand_key="QWENCLOUD_API_KEY",
            fallback_key="DASHSCOPE_API_KEY",
            rerank_base="QWENCLOUD_API_BASE_RERANK",
            image_base="QWENCLOUD_API_BASE_IMAGE",
        )
    if family == "qwen_ai_platform":
        return EnvNames(
            base="QWEN_AI_PLATFORM_API_BASE",
            brand_key="QWEN_AI_PLATFORM_API_KEY",
            fallback_key="DASHSCOPE_API_KEY",
            rerank_base="QWEN_AI_PLATFORM_API_BASE_RERANK",
            image_base="QWEN_AI_PLATFORM_API_BASE_IMAGE",
        )
    return EnvNames(
        base="DASHSCOPE_API_BASE",
        brand_key="",
        fallback_key="DASHSCOPE_API_KEY",
        rerank_base="DASHSCOPE_API_BASE_RERANK",
        image_base="DASHSCOPE_API_BASE_IMAGE",
    )


def apply_env(family: str, step: FamilyStep) -> None:
    names = env_names(family)
    set_env(names["base"], step.get("base"))
    set_env(names["fallback_key"], step.get("fallback_key"))
    set_env(names["rerank_base"], step.get("rerank_base"))
    set_env(names["image_base"], step.get("image_base"))
    if names["brand_key"] == "":
        return
    set_env(names["brand_key"], step.get("brand_key"))


def family_value(
    family: str, step: FamilyStep, api_base: str | None, api_key: str | None
) -> JsonValue:
    apply_env(family, step)
    image: ImageConfig = get_dashscope_family_image_generation_config(family)
    embedding: EmbeddingUrlConfig = get_dashscope_family_embedding_config(family)
    rerank: RerankUrlConfig = get_dashscope_family_rerank_config(family)
    _model, _provider, key, base = get_llm_provider(
        model=f"{family}{PROBE_MODEL}", api_base=api_base, api_key=api_key
    )
    return {
        "chat": {"api_base": base, "api_key": key},
        "embedding_url": embedding.get_complete_url(
            api_base, None, EMBEDDING_MODEL, {}, {}
        ),
        "rerank_url": rerank.get_complete_url(api_base, RERANK_MODEL, None),
        "image": {
            "url": image.get_complete_url(api_base, None, IMAGE_MODEL, {}, {}),
            "headers": image.validate_environment(
                {}, IMAGE_MODEL, NoMessages(), {}, {}, None
            ),
        },
    }


def family_settings(input_value: FamilySettingsInput) -> JsonValue:
    api_base = input_value.get("api_base")
    api_key = input_value.get("api_key")
    return {
        "dashscope": family_value(
            "dashscope", input_value["dashscope"], api_base, api_key
        ),
        "qwencloud": family_value(
            "qwencloud", input_value["qwencloud"], api_base, api_key
        ),
        "qwen_ai_platform": family_value(
            "qwen_ai_platform", input_value["qwen_ai_platform"], api_base, api_key
        ),
    }


def key_message(
    family: str, api_key: str | None, brand_key: str | None, dashscope_key: str | None
) -> JsonValue:
    set_env("DASHSCOPE_API_KEY", dashscope_key)
    set_env("QWENCLOUD_API_KEY", None)
    set_env("QWEN_AI_PLATFORM_API_KEY", None)
    if family == "qwencloud":
        set_env("QWENCLOUD_API_KEY", brand_key)
    if family == "qwen_ai_platform":
        set_env("QWEN_AI_PLATFORM_API_KEY", brand_key)
    return {
        "key": resolve_dashscope_family_api_key(family, api_key),
        "message": missing_dashscope_family_key_message(family),
    }


def family_key_and_message(input_value: FamilyKeyAndMessageInput) -> JsonValue:
    api_key = input_value.get("api_key")
    dashscope_key = input_value.get("dashscope_key")
    return {
        "dashscope": key_message("dashscope", api_key, None, dashscope_key),
        "qwencloud": key_message(
            "qwencloud", api_key, input_value.get("qwencloud_key"), dashscope_key
        ),
        "qwen_ai_platform": key_message(
            "qwen_ai_platform",
            api_key,
            input_value.get("qwen_ai_platform_key"),
            dashscope_key,
        ),
    }
