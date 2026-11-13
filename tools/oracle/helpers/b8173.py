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
    def get_supported_openai_params(self, model: str) -> list[JsonValue]: ...

    def map_openai_params(
        self,
        non_default_params: JsonObject,
        optional_params: JsonObject,
        model: str,
        *,
        drop_params: bool,
    ) -> JsonObject: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: JsonObject,
        litellm_params: JsonObject,
    ) -> str: ...


class MinimaxConfig(ChatConfig, Protocol):
    @staticmethod
    def get_api_base(api_base: str | None = None) -> str: ...

    @staticmethod
    def get_api_key(api_key: str | None = None) -> str | None: ...


class ModelEntry(TypedDict):
    litellm_provider: str
    mode: str
    supports_reasoning: bool
    supports_adaptive_thinking: bool
    input_cost_per_token: float
    output_cost_per_token: float


AI21: ChatConfig = AI21ChatConfig()
MINIMAX: MinimaxConfig = MinimaxChatConfig()
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


def register(provider: str, model: str, *, reasoning: bool, adaptive: bool) -> None:
    entry: ModelEntry = {
        "litellm_provider": provider,
        "mode": "chat",
        "supports_reasoning": reasoning,
        "supports_adaptive_thinking": adaptive,
        "input_cost_per_token": 0.000001,
        "output_cost_per_token": 0.000002,
    }
    cost_map: dict[str, ModelEntry] = {model: entry}
    litellm.register_model(cost_map)


def optional_params(step: Ai21Map | TencentMap) -> JsonObject:
    given = step.get("optional_params")
    if given is None:
        return {}
    return given


def tencent_map(step: TencentMap) -> JsonObject:
    register(
        "tencent",
        step["model"],
        reasoning=step["supports_reasoning"],
        adaptive=step.get("supports_adaptive_thinking") is True,
    )
    return TENCENT.map_openai_params(
        step["params"], optional_params(step), step["model"], drop_params=False
    )


def answer(step: Step) -> JsonValue:
    result: JsonValue
    if step["kind"] == "ai21_params":
        result = AI21.get_supported_openai_params("")
    elif step["kind"] == "ai21_map":
        result = AI21.map_openai_params(
            step["params"], optional_params(step), "", drop_params=False
        )
    elif step["kind"] == "minimax_params":
        register(
            "minimax",
            step["model"],
            reasoning=step["supports_reasoning"],
            adaptive=False,
        )
        result = MINIMAX.get_supported_openai_params(step["model"])
    elif step["kind"] == "minimax_info":
        litellm.api_key = step.get("global_key")
        result = {
            "base": MINIMAX.get_api_base(step.get("api_base")),
            "key": MINIMAX.get_api_key(step.get("api_key")),
        }
    elif step["kind"] == "minimax_url":
        result = MINIMAX.get_complete_url(step.get("api_base"), None, "", {}, {})
    elif step["kind"] == "modelscope_url":
        result = MODELSCOPE.get_complete_url(step.get("api_base"), None, "", {}, {})
    elif step["kind"] == "tencent_params":
        register(
            "tencent",
            step["model"],
            reasoning=step["supports_reasoning"],
            adaptive=False,
        )
        result = TENCENT.get_supported_openai_params(step["model"])
    elif step["kind"] == "tencent_url":
        result = TENCENT.get_complete_url(step.get("api_base"), None, "", {}, {})
    elif step["kind"] == "tencent_map":
        result = tencent_map(step)
    else:
        usage = Usage(
            prompt_tokens=step["prompt_tokens"],
            completion_tokens=step["completion_tokens"],
            total_tokens=step["prompt_tokens"] + step["completion_tokens"],
        )
        prompt, completion = cost_calculator.cost_per_token(step["model"], usage)
        result = {"ok": True, "prompt": f"{prompt:g}", "completion": f"{completion:g}"}
    return result


def run(input_value: Input) -> JsonValue:
    steps = input_value["steps"]
    return [answer(step) for step in steps]
