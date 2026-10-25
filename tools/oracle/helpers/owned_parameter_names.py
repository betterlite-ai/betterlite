from dataclasses import dataclass
from typing import Literal, TypedDict

from litellm.types.litellm_params import (
    AgenticLoopOptions,
    AgenticLoopState,
    BedrockBatchConnection,
    ConnectionSettings,
    InternalState,
    LiteLLMOptions,
    ObservabilityOptions,
    owned_wire_names,
    wire,
    wire_names,
)
from litellm.types.utils import (
    CustomPricingLiteLLMParams,
    StandardCallbackDynamicParams,
    all_litellm_params,
    is_litellm_owned_kwarg,
)

OwnerName = Literal[
    "connection_settings",
    "litellm_options",
    "internal_state",
    "observability",
    "agentic_loop_state",
    "agentic_loop_options",
    "bedrock_batch",
    "standard_callback_dynamic_params",
    "custom_pricing_litellm_params",
]
RootName = Literal["connection_settings", "litellm_options", "internal_state", "invalid_leaf"]


@dataclass(frozen=True)
class InvalidLeaf:
    value: str


class ProjectionInput(TypedDict):
    function: Literal["wire_name_projection"]
    name: str
    owner: OwnerName
    root: RootName | None


class NamesInput(TypedDict):
    function: Literal["owned_name_registry", "internal_prefix_membership"]
    names: list[str]


class SubsetInput(TypedDict):
    function: Literal["subset_registries"]


OWNERS: dict[OwnerName, type] = {
    "connection_settings": ConnectionSettings,
    "litellm_options": LiteLLMOptions,
    "internal_state": InternalState,
    "observability": ObservabilityOptions,
    "agentic_loop_state": AgenticLoopState,
    "agentic_loop_options": AgenticLoopOptions,
    "bedrock_batch": BedrockBatchConnection,
    "standard_callback_dynamic_params": StandardCallbackDynamicParams,
    "custom_pricing_litellm_params": CustomPricingLiteLLMParams,
}


def owner(name: OwnerName) -> type:
    return OWNERS[name]


def root(name: RootName) -> type:
    if name == "invalid_leaf":
        return InvalidLeaf
    return owner(name)


def projection(input_value: ProjectionInput) -> dict[str, object]:
    result: dict[str, object] = {
        "wire": dict(wire(input_value["name"])),
        "names": list(wire_names(owner(input_value["owner"]))),
    }
    if input_value["root"] is not None:
        result["owned"] = list(owned_wire_names(root(input_value["root"])))
    else:
        result["owned"] = None
    return result


def subset() -> dict[str, object]:
    return {
        "agentic_loop": [
            *wire_names(AgenticLoopState),
            *wire_names(AgenticLoopOptions),
        ],
        "bedrock_batch": list(wire_names(BedrockBatchConnection)),
    }


def run(input_value: ProjectionInput | NamesInput | SubsetInput) -> dict[str, object]:
    if input_value["function"] == "wire_name_projection":
        return projection(input_value)
    if input_value["function"] == "owned_name_registry":
        return {
            "names": list(all_litellm_params),
            "owned": [is_litellm_owned_kwarg(name) for name in input_value["names"]],
        }
    if input_value["function"] == "internal_prefix_membership":
        return {
            "owned": [is_litellm_owned_kwarg(name) for name in input_value["names"]]
        }
    return subset()
