from typing import Literal, Protocol, TypedDict

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.types.llms.bedrock import (
    GuardrailConfigBlock,
    PerformanceConfigBlock,
    ServiceTierBlock,
)
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class Flags(TypedDict):
    function: Literal["flags"]
    kind: Literal["nova2", "gpt", "minmax", "web_search", "sampling"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class Handle(TypedDict):
    function: Literal["handle"]
    model_cost: dict[str, JsonValue]
    environment: dict[str, str]
    model: str
    effort: str
    optional_params: dict[str, JsonValue]


class Params(TypedDict):
    function: Literal["params"]
    model_cost: dict[str, JsonValue]
    models: list[str]


class Clamp(TypedDict):
    function: Literal["clamp"]
    environment: dict[str, str]
    cases: list[dict[str, JsonValue]]


class Arguments(TypedDict, total=False):
    maxTokens: int | None
    stopSequences: list[str] | None
    temperature: int | None
    topP: int | None
    topK: int | None


class Defaults(TypedDict):
    function: Literal["defaults"]
    arguments: Arguments


Input = Flags | Handle | Params | Clamp | Defaults


class GroundingTool(TypedDict):
    name: str


# LiteLLM's BedrockToolBlock carries a bare dict in inputSchema; the view keeps the grounding key.
class Grounding(TypedDict, total=False):
    systemTool: GroundingTool | None


class AutoChoice(TypedDict, total=False):
    auto: dict[str, JsonValue]


# Both class methods have untyped bodies upstream, so the protocol carries their signatures.
class Declared(Protocol):
    @classmethod
    def get_config(cls) -> dict[str, JsonValue]: ...

    @classmethod
    def get_config_blocks(
        cls,
    ) -> dict[str, type[GuardrailConfigBlock | PerformanceConfigBlock | ServiceTierBlock]]: ...


# The two helpers whose upstream return types carry a bare dict; the view restates them.
class ConverseSurface(Protocol):
    def _map_web_search_options(
        self, web_search_options: dict[str, JsonValue], model: str
    ) -> Grounding | None: ...

    @staticmethod
    def _auto_tool_choice() -> AutoChoice: ...


DECLARED: type[Declared] = AmazonConverseConfig


# The typed helpers are reachable through a subclass, which is the only seam ruff and pyright allow.
class ConverseConfig(AmazonConverseConfig, ConverseSurface):
    def is_nova_2(self, model: str) -> bool:
        return self._is_nova_2_model(model)

    def is_gpt_reasoning(self, model: str) -> bool:
        return self._is_openai_gpt_reasoning_model(model)

    def needs_min_max_tokens(self, model: str) -> bool:
        return self._requires_min_max_tokens(model)

    def supports_sampling(self, model: str) -> bool:
        return self._supports_sampling_params(model)

    def supported(self, model: str) -> list[str]:
        return self.get_supported_openai_params(model)

    def apply_effort(self, model: str, effort: str, optional_params: dict[str, JsonValue]) -> None:
        self._handle_reasoning_effort_parameter(model, effort, optional_params)

    def clamp_budget(self, optional_params: dict[str, JsonValue]) -> None:
        self._clamp_thinking_budget_tokens(optional_params)

    def grounding(self, model: str) -> Grounding | None:
        surface: ConverseSurface = self
        return surface._map_web_search_options({}, model)

    def auto_tool_choice(self) -> JsonValue:
        surface: ConverseSurface = self
        return JSON.validate_python(surface._auto_tool_choice())


def flags(call: Flags) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    adapter = ConverseConfig()
    kind = call["kind"]
    results: list[JsonValue] = []
    for model in call["models"]:
        if kind == "nova2":
            results.append(adapter.is_nova_2(model))
        elif kind == "gpt":
            results.append(adapter.is_gpt_reasoning(model))
        elif kind == "minmax":
            results.append(adapter.needs_min_max_tokens(model))
        elif kind == "sampling":
            results.append(adapter.supports_sampling(model))
        else:
            results.append(JSON.validate_python(adapter.grounding(model)))
    return {"results": results}


def handle(call: Handle) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    adapter = ConverseConfig()
    optional_params = PARAMS.validate_python(call["optional_params"])
    adapter.apply_effort(call["model"], call["effort"], optional_params)
    return {"params": optional_params}


def params(call: Params) -> JsonValue:
    litellm.model_cost = call["model_cost"]
    adapter = ConverseConfig()
    results: list[JsonValue] = [
        JSON.validate_python(adapter.supported(model)) for model in call["models"]
    ]
    return {"results": results, "auto": adapter.auto_tool_choice()}


def clamp(call: Clamp) -> JsonValue:
    adapter = ConverseConfig()
    results: list[JsonValue] = []
    for values in call["cases"]:
        optional_params = PARAMS.validate_python(values)
        adapter.clamp_budget(optional_params)
        results.append(optional_params)
    return {"results": results}


def defaults(call: Defaults) -> JsonValue:
    instance = AmazonConverseConfig(**call["arguments"])
    encoded: dict[str, JsonValue] = {}
    for name, value in DECLARED.get_config().items():
        encoded[name] = (
            {"kind": "property"}
            if name == "custom_llm_provider"
            else JSON.validate_python(value)
        )
    blocks: dict[str, JsonValue] = {}
    for name, block in DECLARED.get_config_blocks().items():
        blocks[name] = block.__name__
    return {
        "config": encoded,
        "custom_llm_provider": instance.custom_llm_provider,
        "config_blocks": blocks,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "flags":
        return flags(input_value)
    if input_value["function"] == "handle":
        return handle(input_value)
    if input_value["function"] == "params":
        return params(input_value)
    if input_value["function"] == "clamp":
        return clamp(input_value)
    return defaults(input_value)
