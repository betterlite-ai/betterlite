import io
from collections.abc import Callable
from contextlib import redirect_stdout
from typing import NotRequired, TypeAlias, TypedDict

import litellm
import litellm.utils as litellm_utils
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Scalar: TypeAlias = None | bool | int | float | str
ModelCost: TypeAlias = dict[str, dict[str, Scalar]]
Capability: TypeAlias = Callable[[str, str | None], bool]


class Step(TypedDict):
    function: str
    model: str
    custom_llm_provider: NotRequired[str | None]
    key: NotRequired[str]


class Fixture(TypedDict):
    model_cost: NotRequired[ModelCost]


class Input(TypedDict):
    fixture: NotRequired[Fixture]
    steps: list[Step]


INPUT = TypeAdapter(Input)

CAPABILITIES: dict[str, Capability] = {
    "supports_system_messages": litellm_utils.supports_system_messages,
    "supports_parallel_function_calling": litellm_utils.supports_parallel_function_calling,
    "supports_function_calling": litellm_utils.supports_function_calling,
    "supports_tool_choice": litellm_utils.supports_tool_choice,
    "supports_prompt_caching": litellm_utils.supports_prompt_caching,
    "supports_reasoning": litellm_utils.supports_reasoning,
    "supports_none_reasoning_effort": litellm_utils.supports_none_reasoning_effort,
    "supports_mid_conversation_system": litellm_utils.supports_mid_conversation_system,
    "supports_native_structured_output": litellm_utils.supports_native_structured_output,
    "supports_vision": litellm_utils.supports_vision,
    "supports_web_search": litellm_utils.supports_web_search,
    "supports_pdf_input": litellm_utils.supports_pdf_input,
}


def model_cost_of(fixture: Fixture | None) -> ModelCost:
    if fixture is None:
        return {}
    costs = fixture.get("model_cost")
    return {} if costs is None else costs


def require_key(step: Step) -> str:
    key = step.get("key")
    if key is None:
        raise ValueError(f"the {step['function']} function needs a key")
    return key


def capability_call(step: Step) -> Capability:
    key = require_key(step)
    if key not in CAPABILITIES:
        raise ValueError(f"no named wrapper for capability key: {key}")
    return CAPABILITIES[key]


def run_step(step: Step) -> Json:
    function = step["function"]
    model = step["model"]
    provider = step.get("custom_llm_provider")
    if function == "supports_native_streaming":
        return litellm_utils.supports_native_streaming(model, provider)
    if function == "supports_response_schema":
        return litellm_utils.supports_response_schema(model, provider)
    if function == "declared_value_factory":
        return litellm_utils.declared_value_factory(model, provider, require_key(step))
    if function == "is_explicitly_disabled_factory":
        return litellm_utils.is_explicitly_disabled_factory(model, provider, require_key(step))
    if function == "supports_factory":
        return capability_call(step)(model, provider)
    if function in CAPABILITIES:
        return CAPABILITIES[function](model, provider)
    raise ValueError(f"unknown capability function: {function}")


def run(input_value: Json) -> Json:
    parsed = INPUT.validate_python(input_value)
    litellm.model_cost = model_cost_of(parsed.get("fixture"))
    with redirect_stdout(io.StringIO()):
        return [run_step(step) for step in parsed["steps"]]
