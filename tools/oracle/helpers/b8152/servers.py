import os
from typing import Literal, TypedDict, cast

import litellm
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.docker_model_runner.chat.transformation import DockerModelRunnerChatConfig
from litellm.llms.lemonade.cost_calculator import cost_per_token
from litellm.llms.lm_studio.chat.transformation import LMStudioChatConfig
from litellm.llms.lm_studio.embed.transformation import LmStudioEmbeddingConfig
from litellm.llms.xinference.image_generation.transformation import (
    XInferenceImageGenerationConfig as XinferenceConfig,
)
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import Usage
from pydantic import JsonValue

NO_PARAMS: dict[str, JsonValue] = {}
DOCKER_LITELLM_PARAMS: dict[str, JsonValue] = {"custom_llm_provider": "docker_model_runner"}


class ProviderCall(TypedDict):
    provider: Literal["docker_model_runner", "llamafile", "lm_studio"]
    api_base: str | None
    api_key: str | None
    env: dict[str, str]


class ParamsCall(TypedDict):
    provider: Literal["docker_model_runner", "lm_studio"]
    model: str
    params: list[tuple[str, JsonValue]]
    preset: dict[str, JsonValue]
    chat_models: list[str]
    text_models: list[str]


class DockerCall(TypedDict):
    api_base: str | None
    model: str
    messages: list[dict[str, JsonValue]]


class XinferenceCall(TypedDict):
    model: str
    params: dict[str, JsonValue]
    preset: dict[str, JsonValue]
    drop_params: bool


class CostCall(TypedDict):
    model: str
    prompt_tokens: int
    completion_tokens: int


def apply_env(env: dict[str, str]) -> None:
    os.environ.clear()
    for name, value in env.items():
        os.environ[name] = value


def provider_value(call: ProviderCall) -> JsonValue:
    apply_env(call["env"])
    _, _, api_key, api_base = get_llm_provider(
        model=f"{call['provider']}/m", api_base=call["api_base"], api_key=call["api_key"]
    )
    return {"api_base": api_base, "api_key": api_key}


def params_value(call: ParamsCall) -> JsonValue:
    litellm.open_ai_chat_completion_models = set(call["chat_models"])
    litellm.open_ai_text_completion_models = set(call["text_models"])
    if call["provider"] == "docker_model_runner":
        config: DockerModelRunnerChatConfig | LMStudioChatConfig = DockerModelRunnerChatConfig()
    else:
        config = LMStudioChatConfig()
    non_default: dict[str, JsonValue] = dict(call["params"])
    optional: dict[str, JsonValue] = dict(call["preset"])
    mapped = config.map_openai_params(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/docker_model_runner/chat/transformation.py#L116-L135
        non_default_params=non_default,
        optional_params=optional,
        model=call["model"],
        drop_params=False,
    )
    return cast("dict[str, JsonValue]", mapped)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/docker_model_runner/chat/transformation.py#L116-L135


def docker_value(config: DockerModelRunnerChatConfig, call: DockerCall) -> JsonValue:
    url = config.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/docker_model_runner/chat/transformation.py#L67-L75
        api_base=call["api_base"],
        api_key=None,
        model=call["model"],
        optional_params=NO_PARAMS,
        litellm_params=NO_PARAMS,
    )
    optional: dict[str, JsonValue] = {}
    try:
        body = config.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L476-L511
            model=call["model"],
            messages=cast("list[AllMessageValues]", call["messages"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998
            optional_params=optional,
            litellm_params=DOCKER_LITELLM_PARAMS,
            headers=NO_PARAMS,
        )
    except (KeyError, TypeError, ValueError):
        return {"url": url, "ok": False}
    return {"url": url, "ok": True, "messages": cast("dict[str, JsonValue]", body)["messages"]}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L506-L511


def embed_value() -> JsonValue:
    config = LmStudioEmbeddingConfig()
    supported: list[JsonValue] = [*config.get_supported_openai_params()]
    return {
        "supported": supported,
        "config": cast("dict[str, JsonValue]", config.get_config()),  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/lm_studio/embed/transformation.py#L25-L39
    }


def xinference_value(config: XinferenceConfig, call: XinferenceCall) -> JsonValue:
    supported: list[JsonValue] = [*config.get_supported_openai_params(call["model"])]
    non_default: dict[str, JsonValue] = dict(call["params"])
    optional: dict[str, JsonValue] = dict(call["preset"])
    try:
        mapped = config.map_openai_params(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/xinference/image_generation/transformation.py#L21-L41
            non_default_params=non_default,
            optional_params=optional,
            model=call["model"],
            drop_params=call["drop_params"],
        )
    except ValueError:
        return {"ok": False, "supported": supported}
    return {"ok": True, "supported": supported, "value": cast("dict[str, JsonValue]", mapped)}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/xinference/image_generation/transformation.py#L21-L41


def cost_value(call: CostCall) -> JsonValue:
    prompt, completion = cost_per_token(
        model=call["model"],
        usage=Usage(
            prompt_tokens=call["prompt_tokens"], completion_tokens=call["completion_tokens"]
        ),
    )
    return {
        "model": call["model"],
        "prompt_is_zero": prompt == 0.0,
        "completion_is_zero": completion == 0.0,
    }
