from typing import Literal, Protocol, TypedDict

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.types.llms.bedrock import BedrockToolBlock
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
# _map_web_search_options ignores its first argument, so an empty mapping carries the call.
EMPTY_PARAMS: dict[str, JsonValue] = {}


class Models(TypedDict):
    models: list[str]


class Nova2Models(Models):
    function: Literal["nova2_models"]


class GptModels(Models):
    function: Literal["gpt_models"]


class MinMaxTokens(Models):
    function: Literal["min_max_tokens"]


class WebSearch(Models):
    function: Literal["web_search"]


class ModelPair(TypedDict):
    model: str
    base_model: str


class Handle(TypedDict):
    function: Literal["handle"]
    model_cost: dict[str, JsonValue]
    environment: dict[str, str]
    model: str
    effort: str
    optional_params: dict[str, JsonValue]


class DeepseekFlags(TypedDict):
    function: Literal["deepseek_flags"]
    cases: list[ModelPair]


class ThinkingAccepts(TypedDict):
    function: Literal["thinking_accepts"]
    model_cost: dict[str, JsonValue]
    cases: list[ModelPair]


class Sampling(TypedDict):
    function: Literal["sampling"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class SupportedParams(TypedDict):
    function: Literal["supported_params"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class ClampValues(TypedDict):
    optional_params: dict[str, JsonValue]


class Clamp(TypedDict):
    function: Literal["clamp"]
    environment: dict[str, str]
    cases: list[ClampValues]


class Arguments(TypedDict, total=False):
    maxTokens: int | None
    stopSequences: list[str] | None
    temperature: int | None
    topP: int | None
    topK: int | None


class ClassDefaults(TypedDict):
    function: Literal["class_defaults"]
    arguments: Arguments


class AutoToolChoice(TypedDict):
    function: Literal["auto_tool_choice"]


Input = (
    Nova2Models
    | GptModels
    | MinMaxTokens
    | Handle
    | DeepseekFlags
    | ThinkingAccepts
    | Sampling
    | SupportedParams
    | Clamp
    | WebSearch
    | ClassDefaults
    | AutoToolChoice
)


class BlockClass(Protocol):
    __name__: str


# Both class methods have untyped bodies upstream, so the protocol carries their signatures.
class Declared(Protocol):
    @classmethod
    def get_config(cls) -> dict[str, JsonValue | property]: ...
    @classmethod
    def get_config_blocks(cls) -> dict[str, type[BlockClass]]: ...


# These three helpers take or return a bare dict upstream; the protocol restates them.
class ConverseSurface(Protocol):
    def _handle_reasoning_effort_parameter(  # ast-grep-ignore: no-long-names
        self, model: str, reasoning_effort: str, optional_params: dict[str, JsonValue]
    ) -> None: ...

    def _clamp_thinking_budget_tokens(self, optional_params: dict[str, JsonValue]) -> None: ...

    def _map_web_search_options(
        self, web_search_options: dict[str, JsonValue], model: str
    ) -> BedrockToolBlock | None: ...

    def apply_effort(
        self, model: str, effort: str, optional_params: dict[str, JsonValue]
    ) -> None:
        self._handle_reasoning_effort_parameter(model, effort, optional_params)

    def clamp_budget(self, optional_params: dict[str, JsonValue]) -> None:
        self._clamp_thinking_budget_tokens(optional_params)

    def grounding(self, model: str) -> BedrockToolBlock | None:
        return self._map_web_search_options(EMPTY_PARAMS, model)


DECLARED: Declared = AmazonConverseConfig


# The typed helpers are reachable through a subclass, which is the only seam ruff and pyright allow.
class ConverseConfig(AmazonConverseConfig, ConverseSurface):
    def is_nova_2(self, model: str) -> bool:
        return self._is_nova_2_model(model)

    def is_gpt_reasoning(self, model: str) -> bool:
        return self._is_openai_gpt_reasoning_model(model)

    def needs_min_max_tokens(self, model: str) -> bool:
        return self._requires_min_max_tokens(model)

    def is_deepseek(self, model: str, base_model: str) -> bool:
        return self._is_deepseek_model(model, base_model)

    def rejects_reasoning_effort(self, model: str, base_model: str) -> bool:
        return self._model_rejects_reasoning_effort_param(model, base_model)

    def accepts_thinking(self, model: str, base_model: str) -> bool:
        return self._model_accepts_anthropic_thinking_param(model, base_model)

    def supports_sampling(self, model: str) -> bool:
        return self._supports_sampling_params(model)

    def supported(self, model: str) -> list[str]:
        return self.get_supported_openai_params(model)

    def auto_tool_choice(self) -> JsonValue:
        return JSON.validate_python(self._auto_tool_choice())


def flag_results(flags: list[bool]) -> JsonValue:
    results: list[JsonValue] = []
    for flag in flags:
        results.append(flag)
    return {"results": results}


def nova2_models(call: Nova2Models) -> JsonValue:
    adapter = ConverseConfig()
    return flag_results([adapter.is_nova_2(model) for model in call["models"]])


def gpt_models(call: GptModels) -> JsonValue:
    adapter = ConverseConfig()
    return flag_results([adapter.is_gpt_reasoning(model) for model in call["models"]])


def min_max_tokens(call: MinMaxTokens) -> JsonValue:
    adapter = ConverseConfig()
    return flag_results([adapter.needs_min_max_tokens(model) for model in call["models"]])


def handle(call: Handle) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    adapter = ConverseConfig()
    optional_params = PARAMS.validate_python(call["optional_params"])
    adapter.apply_effort(call["model"], call["effort"], optional_params)
    return {"params": optional_params}


def deepseek_flags(call: DeepseekFlags) -> JsonValue:
    adapter = ConverseConfig()
    results: list[JsonValue] = []
    for case in call["cases"]:
        results.append(
            {
                "deepseek": adapter.is_deepseek(case["model"], case["base_model"]),
                "rejects": adapter.rejects_reasoning_effort(
                    case["model"], case["base_model"]
                ),
            }
        )
    return {"results": results}


def thinking_accepts(call: ThinkingAccepts) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    adapter = ConverseConfig()
    return flag_results(
        [
            adapter.accepts_thinking(case["model"], case["base_model"])
            for case in call["cases"]
        ]
    )


def sampling(call: Sampling) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    adapter = ConverseConfig()
    return flag_results([adapter.supports_sampling(model) for model in call["models"]])


def supported_params(call: SupportedParams) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    adapter = ConverseConfig()
    results: list[JsonValue] = []
    for model in call["models"]:
        results.append(JSON.validate_python(adapter.supported(model)))
    return {"results": results}


def clamp(call: Clamp) -> JsonValue:
    adapter = ConverseConfig()
    results: list[JsonValue] = []
    for values in call["cases"]:
        optional_params = PARAMS.validate_python(values["optional_params"])
        adapter.clamp_budget(optional_params)
        results.append(optional_params)
    return {"results": results}


def web_search(call: WebSearch) -> JsonValue:
    adapter = ConverseConfig()
    results: list[JsonValue] = []
    for model in call["models"]:
        results.append(JSON.validate_python(adapter.grounding(model)))
    return {"results": results}


def class_defaults(call: ClassDefaults) -> JsonValue:
    instance = AmazonConverseConfig(**call["arguments"])
    encoded: dict[str, JsonValue] = {}
    for name, value in DECLARED.get_config().items():
        match value:
            case property():
                encoded[name] = {"kind": "property"}
            case None | bool() | int() | float() | str() | list() | dict():
                encoded[name] = JSON.validate_python(value)
    blocks: dict[str, JsonValue] = {}
    for name, block in DECLARED.get_config_blocks().items():
        blocks[name] = block.__name__
    return {
        "config": encoded,
        "custom_llm_provider": instance.custom_llm_provider,
        "config_blocks": blocks,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "nova2_models":
        return nova2_models(input_value)
    if input_value["function"] == "gpt_models":
        return gpt_models(input_value)
    if input_value["function"] == "min_max_tokens":
        return min_max_tokens(input_value)
    if input_value["function"] == "handle":
        return handle(input_value)
    if input_value["function"] == "deepseek_flags":
        return deepseek_flags(input_value)
    if input_value["function"] == "thinking_accepts":
        return thinking_accepts(input_value)
    if input_value["function"] == "sampling":
        return sampling(input_value)
    if input_value["function"] == "supported_params":
        return supported_params(input_value)
    if input_value["function"] == "clamp":
        return clamp(input_value)
    if input_value["function"] == "web_search":
        return web_search(input_value)
    if input_value["function"] == "class_defaults":
        return class_defaults(input_value)
    return ConverseConfig().auto_tool_choice()
