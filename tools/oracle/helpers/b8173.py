from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict

import litellm
from litellm.llms.ai21.chat.transformation import AI21ChatConfig
from litellm.llms.minimax.chat.transformation import MinimaxChatConfig
from litellm.llms.modelscope.chat.transformation import ModelScopeChatConfig
from litellm.llms.tencent import cost_calculator
from litellm.llms.tencent.chat.transformation import TencentChatConfig
from litellm.types.utils import Usage
from pydantic import JsonValue

JsonObject: TypeAlias = dict[str, JsonValue]


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        drop_params: bool,
    ) -> JsonObject: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: JsonObject,
        litellm_params: JsonObject,
        stream: bool | None = None,
    ) -> str: ...


AI21: ChatConfig = AI21ChatConfig()
MINIMAX: ChatConfig = MinimaxChatConfig()
MODELSCOPE: ChatConfig = ModelScopeChatConfig()
TENCENT: ChatConfig = TencentChatConfig()


class Ai21Params(TypedDict):
    kind: Literal["ai21_params"]


class Ai21Map(TypedDict):
    kind: Literal["ai21_map"]
    params: JsonObject
    optional_params: NotRequired[JsonObject]


class MinimaxParams(TypedDict):
    kind: Literal["minimax_params"]
    model: str
    supports_reasoning: bool


class MinimaxInfo(TypedDict):
    kind: Literal["minimax_info"]
    api_base: NotRequired[str]
    env_base: NotRequired[str]
    api_key: NotRequired[str]
    env_key: NotRequired[str]
    global_key: NotRequired[str]


class MinimaxUrl(TypedDict):
    kind: Literal["minimax_url"]
    api_base: NotRequired[str]
    env_base: NotRequired[str]


class ModelscopeUrl(TypedDict):
    kind: Literal["modelscope_url"]
    api_base: NotRequired[str]


class TencentParams(TypedDict):
    kind: Literal["tencent_params"]
    model: str
    supports_reasoning: bool


class TencentUrl(TypedDict):
    kind: Literal["tencent_url"]
    api_base: NotRequired[str]


class TencentMap(TypedDict):
    kind: Literal["tencent_map"]
    model: str
    supports_reasoning: bool
    supports_adaptive_thinking: NotRequired[bool]
    params: JsonObject
    optional_params: NotRequired[JsonObject]


class TencentCost(TypedDict):
    kind: Literal["tencent_cost"]
    model: str
    prompt_tokens: int
    completion_tokens: int


Step: TypeAlias = (
    Ai21Params
    | Ai21Map
    | MinimaxParams
    | MinimaxInfo
    | MinimaxUrl
    | ModelscopeUrl
    | TencentParams
    | TencentUrl
    | TencentMap
    | TencentCost
)


class Input(TypedDict):
    steps: list[Step]


def register(provider: str, model: str, reasoning: bool, adaptive: bool) -> None:
    litellm.register_model(
        {
            model: {
                "litellm_provider": provider,
                "mode": "chat",
                "supports_reasoning": reasoning,
                "supports_adaptive_thinking": adaptive,
                "input_cost_per_token": 0.000001,
                "output_cost_per_token": 0.000002,
            }
        }
    )


def mapped_params(config: ChatConfig, step: TencentMap, optional: JsonObject) -> JsonObject:
    register(
        "tencent",
        step["model"],
        step["supports_reasoning"],
        step.get("supports_adaptive_thinking") is True,
    )
    return config.map_openai_params(step["params"], optional, step["model"], False)


def answer(step: Step) -> JsonValue:
    if step["kind"] == "ai21_params":
        return AI21.get_supported_openai_params("")
    if step["kind"] == "ai21_map":
        optional = step.get("optional_params")
        return AI21.map_openai_params(
            step["params"], {} if optional is None else optional, "", False
        )
    if step["kind"] == "minimax_params":
        register("minimax", step["model"], step["supports_reasoning"], False)
        return MINIMAX.get_supported_openai_params(step["model"])
    if step["kind"] == "minimax_info":
        litellm.api_key = step.get("global_key")
        return {
            "base": MinimaxChatConfig.get_api_base(step.get("api_base")),
            "key": MinimaxChatConfig.get_api_key(step.get("api_key")),
        }
    if step["kind"] == "minimax_url":
        return MINIMAX.get_complete_url(step.get("api_base"), None, "", {}, {})
    if step["kind"] == "modelscope_url":
        return MODELSCOPE.get_complete_url(step.get("api_base"), None, "", {}, {})
    if step["kind"] == "tencent_params":
        register("tencent", step["model"], step["supports_reasoning"], False)
        return TENCENT.get_supported_openai_params(step["model"])
    if step["kind"] == "tencent_url":
        return TENCENT.get_complete_url(step.get("api_base"), None, "", {}, {})
    if step["kind"] == "tencent_map":
        optional = step.get("optional_params")
        return mapped_params(TENCENT, step, {} if optional is None else optional)
    usage = Usage(
        prompt_tokens=step["prompt_tokens"],
        completion_tokens=step["completion_tokens"],
        total_tokens=step["prompt_tokens"] + step["completion_tokens"],
    )
    prompt, completion = cost_calculator.cost_per_token(step["model"], usage)
    return {"ok": True, "prompt": f"{prompt:g}", "completion": f"{completion:g}"}


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = []
    for step in input_value["steps"]:
        results.append(answer(step))
    return results
