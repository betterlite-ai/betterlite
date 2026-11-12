import logging
import os
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
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
Pair: TypeAlias = tuple[str, Json]
Host: TypeAlias = Literal["baseten", "cerebras", "featherless"]


class BasetenSnapshot(TypedDict, total=False):
    max_tokens: int | None
    response_format: dict[str, Json] | None
    seed: int | None
    stop: list[Json] | None
    stream: bool | None
    temperature: float | None
    top_p: int | None
    tool_choice: str | None
    tools: list[Json] | None
    user: str | None
    presence_penalty: int | None
    frequency_penalty: int | None
    stream_options: dict[str, Json] | None


class CerebrasSnapshot(TypedDict, total=False):
    max_tokens: int | None
    response_format: dict[str, Json] | None
    seed: int | None
    stop: str | None
    stream: bool | None
    temperature: float | None
    top_p: int | None
    tool_choice: str | None
    tools: list[Json] | None
    user: str | None
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
    host: Host
    model: str
    config: dict[str, Json]
    params: list[Pair]
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
BASETEN_SNAPSHOT = TypeAdapter(BasetenSnapshot)
CEREBRAS_SNAPSHOT = TypeAdapter(CerebrasSnapshot)
FEATHERLESS_SNAPSHOT = TypeAdapter(FeatherlessSnapshot)


def build(
    host: Host, fields: dict[str, Json]
) -> BasetenConfig | CerebrasConfig | FeatherlessAIConfig:
    if host == "baseten":
        return BasetenConfig(**BASETEN_SNAPSHOT.validate_python(fields))
    if host == "cerebras":
        return CerebrasConfig(**CEREBRAS_SNAPSHOT.validate_python(fields))
    return FeatherlessAIConfig(**FEATHERLESS_SNAPSHOT.validate_python(fields))


def map_value(step: MapStep) -> Json:
    if step["host"] == "cerebras":
        entry: dict[str, Json] = {"supports_reasoning": step.get("supports_reasoning") is True}
        litellm.model_cost[f"cerebras/{step['model']}"] = entry  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    config = build(step["host"], step["config"])
    exported = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L73
        "dict[str, Json]",
        config.get_config(),  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L73
    )
    options: dict[str, Json] = {}
    params: dict[str, Json] = dict(step["params"])
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
    except Exception:  # noqa: BLE001  # the Featherless mapper's only rejection is UnsupportedParamsError with status 400  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/featherless_ai/chat/transformation.py#L85-L95
        return {"ok": False, "error": {"status": 400}}
    pairs: list[Json] = [list(pair) for pair in mapped.items()]
    baseten: Json = None
    if step["host"] == "baseten":
        config = BasetenConfig()
        baseten = {
            "model": step["model"],
            "dedicated": config.is_dedicated_deployment(step["model"]),
            "api_base": config.get_api_base_for_model(step["model"]),
        }
    return {
        "ok": True,
        "value": {"config": exported, "supported": supported, "options": pairs, "baseten": baseten},
    }


def info_value(step: InfoStep) -> Json:
    host = step["host"]
    config: (
        FeatherlessAIConfig
        | AIMLChatConfig
        | EmpowerChatConfig
        | FriendliaiChatConfig
        | GaladrielChatConfig
        | GithubChatConfig
    )
    if host == "featherless":
        config = FeatherlessAIConfig()
    elif host == "aiml":
        config = AIMLChatConfig()
    elif host == "empower":
        config = EmpowerChatConfig()
    elif host == "friendliai":
        config = FriendliaiChatConfig()
    elif host == "galadriel":
        config = GaladrielChatConfig()
    else:
        config = GithubChatConfig()
    base, key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/featherless_ai/chat/transformation.py#L103
        step.get("api_base"), step.get("api_key")
    )
    return {"ok": True, "value": {"base": base, "key": key}}


def step_value(step: MapStep | InfoStep) -> Json:
    if step["op"] == "map":
        return map_value(step)
    return info_value(step)


def scenario_value(scenario: Scenario) -> dict[str, Json]:
    os.environ.clear()
    environment = scenario.get("environment")
    if environment is not None:
        for name, value in environment.items():
            os.environ[name] = value
    litellm.drop_params = drop_params_env_flag(os.environ, logging.getLogger("LiteLLM"))
    steps: list[Json] = [step_value(step) for step in scenario["calls"]]
    return {"steps": steps}


def run(input_value: dict[str, Json]) -> Json:
    parsed = INPUT.validate_python(input_value)
    return [scenario_value(scenario) for scenario in parsed["scenarios"]]
