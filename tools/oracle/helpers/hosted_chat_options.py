import logging
import os
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.exceptions import UnsupportedParamsError
from litellm.litellm_core_utils.core_helpers import drop_params_env_flag
from litellm.llms.aiml.chat.transformation import AIMLChatConfig
from litellm.llms.baseten.chat import BasetenConfig
from litellm.llms.cerebras.chat import CerebrasConfig
from litellm.llms.empower.chat.transformation import EmpowerChatConfig
from litellm.llms.featherless_ai.chat.transformation import FeatherlessAIConfig
from litellm.llms.friendliai.chat.transformation import FriendliaiChatConfig
from litellm.llms.galadriel.chat.transformation import GaladrielChatConfig
from litellm.llms.github.chat.transformation import GithubChatConfig
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class TokenLimitSnapshot(TypedDict, total=False):
    max_tokens: int | None
    response_format: dict[str, Json] | None
    seed: int | None
    stream: bool | None
    top_p: int | None
    tool_choice: str | None
    tools: list[Json] | None
    user: str | None


class BasetenSnapshot(TokenLimitSnapshot, total=False):
    stop: list[Json] | None
    temperature: float | None
    presence_penalty: int | None
    frequency_penalty: int | None
    stream_options: dict[str, Json] | None


class CerebrasSnapshot(TokenLimitSnapshot, total=False):
    stop: str | None
    temperature: float | None
    reasoning_effort: str | None


class FeatherlessSnapshot(TypedDict, total=False):
    frequency_penalty: int | None
    function_call: str | dict[str, Json] | None
    functions: list[Json] | None
    logit_bias: dict[str, Json] | None
    max_tokens: int | None
    n: int | None
    presence_penalty: int | None
    stop: str | list[Json] | None
    temperature: int | None
    top_p: int | None
    response_format: dict[str, Json] | None
    tool_choice: str | None
    tools: list[Json] | None


class MapStep(TypedDict):
    op: Literal["map"]
    host: Literal["baseten", "cerebras", "featherless"]
    model: str
    config: dict[str, Json]
    params: list[tuple[str, Json]]
    drop_params: NotRequired[bool]
    supports_reasoning: NotRequired[bool]


class InfoStep(TypedDict):
    op: Literal["info"]
    host: Literal["featherless", "aiml", "empower", "friendliai", "galadriel", "github"]
    api_base: NotRequired[str]
    api_key: NotRequired[str]


class Scenario(TypedDict):
    environment: NotRequired[dict[str, str]]
    calls: list[MapStep | InfoStep]


class Input(TypedDict):
    scenarios: list[Scenario]


INPUT = TypeAdapter(Input)


def build(
    host: Literal["baseten", "cerebras", "featherless"], fields: dict[str, Json]
) -> BasetenConfig | CerebrasConfig | FeatherlessAIConfig:
    if host == "baseten":
        return BasetenConfig(**TypeAdapter(BasetenSnapshot).validate_python(fields))
    if host == "cerebras":
        return CerebrasConfig(**TypeAdapter(CerebrasSnapshot).validate_python(fields))
    return FeatherlessAIConfig(**TypeAdapter(FeatherlessSnapshot).validate_python(fields))


def map_value(step: MapStep) -> Json:
    if step["host"] == "cerebras":
        entry: dict[str, Json] = {"supports_reasoning": step.get("supports_reasoning") is True}
        litellm.model_cost[f"cerebras/{step['model']}"] = entry  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    config = build(step["host"], step["config"])
    exported = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L73
        "dict[str, Json]",
        config.get_config(),  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L73
    )
    params: dict[str, Json] = dict(step["params"])
    options: dict[str, Json] = {}
    try:
        mapped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L219
            "dict[str, Json]",
            config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L219
                params,
                options,
                step["model"],
                drop_params=step.get("drop_params") is True,
            ),
        )
        supported = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/featherless_ai/chat/transformation.py#L60
            "list[Json]",
            config.get_supported_openai_params(step["model"]),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/featherless_ai/chat/transformation.py#L60
        )
    except UnsupportedParamsError as failure:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L945
        return {"ok": False, "error": {"status": failure.status_code}}
    pairs: list[Json] = [list(pair) for pair in mapped.items()]
    baseten: Json = None
    if step["host"] == "baseten":
        classifier = BasetenConfig()
        baseten = {
            "model": step["model"],
            "dedicated": classifier.is_dedicated_deployment(step["model"]),
            "api_base": classifier.get_api_base_for_model(step["model"]),
        }
    return {
        "ok": True,
        "value": {"config": exported, "supported": supported, "options": pairs, "baseten": baseten},
    }


def info_value(step: InfoStep) -> Json:
    base: str | None
    key: str | None
    if step["host"] == "featherless":
        base, key = FeatherlessAIConfig()._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/featherless_ai/chat/transformation.py#L100
            step.get("api_base"), step.get("api_key")
        )
    elif step["host"] == "aiml":
        base, key = AIMLChatConfig()._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/aiml/chat/transformation.py#L12
            step.get("api_base"), step.get("api_key")
        )
    elif step["host"] == "empower":
        base, key = EmpowerChatConfig()._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/empower/chat/transformation.py#L8
            step.get("api_base"), step.get("api_key")
        )
    elif step["host"] == "friendliai":
        base, key = FriendliaiChatConfig()._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/friendliai/chat/transformation.py#L8
            step.get("api_base"), step.get("api_key")
        )
    elif step["host"] == "galadriel":
        base, key = GaladrielChatConfig()._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/galadriel/chat/transformation.py#L8
            step.get("api_base"), step.get("api_key")
        )
    else:
        base, key = GithubChatConfig()._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/github/chat/transformation.py#L8
            step.get("api_base"), step.get("api_key")
        )
    return {"ok": True, "value": {"base": base, "key": key}}


def run(input_value: dict[str, Json]) -> Json:
    parsed = INPUT.validate_python(input_value)
    scenarios: list[Json] = []
    for scenario in parsed["scenarios"]:
        os.environ.clear()
        environment = scenario.get("environment")
        if environment is not None:
            for name, value in environment.items():
                os.environ[name] = value
        litellm.drop_params = drop_params_env_flag(os.environ, logging.getLogger("LiteLLM"))
        steps: list[Json] = [
            map_value(step) if step["op"] == "map" else info_value(step)
            for step in scenario["calls"]
        ]
        scenarios.append({"steps": steps})
    return scenarios
