from typing import Literal, Protocol, TypedDict

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from litellm.types.llms.bedrock import BedrockToolBlock
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


# Both class methods have untyped bodies upstream, so the protocol carries their signatures.
class Declared(Protocol):
    @classmethod
    def get_config(cls) -> dict[str, JsonValue | property]: ...
    @classmethod
    def get_config_blocks(cls) -> dict[str, type[object]]: ...


# These three helpers take or return a bare dict upstream; the protocol restates them.
class ConverseSurface(Protocol):
    def _handle_reasoning_effort_parameter(  # ast-grep-ignore: no-long-names
        self, model: str, reasoning_effort: str, optional_params: dict[str, JsonValue]
    ) -> None: ...

    def _clamp_thinking_budget_tokens(self, optional_params: dict[str, JsonValue]) -> None: ...

    def _map_web_search_options(
        self, web_search_options: dict[str, JsonValue], model: str
    ) -> BedrockToolBlock | None: ...

    @classmethod
    def _supports_sampling_params(cls, model: str) -> bool: ...

    def apply_effort(
        self, model: str, effort: str, optional_params: dict[str, JsonValue]
    ) -> None:
        self._handle_reasoning_effort_parameter(model, effort, optional_params)

    def clamp_budget(self, optional_params: dict[str, JsonValue]) -> None:
        self._clamp_thinking_budget_tokens(optional_params)

    def grounding(self, model: str) -> BedrockToolBlock | None:
        return self._map_web_search_options({}, model)


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

    def auto_tool_choice(self) -> JsonValue:
        return JSON.validate_python(self._auto_tool_choice())


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
    results: list[JsonValue] = []
    for model in call["models"]:
        results.append(JSON.validate_python(adapter.supported(model)))
    answer: dict[str, JsonValue] = {"results": results, "auto": adapter.auto_tool_choice()}
    return answer


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
    if input_value["function"] == "flags":
        return flags(input_value)
    if input_value["function"] == "handle":
        return handle(input_value)
    if input_value["function"] == "params":
        return params(input_value)
    if input_value["function"] == "clamp":
        return clamp(input_value)
    return defaults(input_value)
