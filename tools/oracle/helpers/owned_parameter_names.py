import types
from typing import Any, cast

from litellm.types.litellm_params import (
    AgenticLoopOptions,
    AgenticLoopState,
    BedrockBatchConnection,
    ConnectionSettings,
    LiteLLMOptions,
    ObservabilityOptions,
    owned_wire_names,
    wire,
    wire_names,
)
from litellm.types.utils import all_litellm_params, is_litellm_owned_kwarg


def owner(name: str) -> type[Any]:
    owners = {
        "connection_settings": ConnectionSettings,
        "litellm_options": LiteLLMOptions,
        "observability": ObservabilityOptions,
        "internal_state": types.SimpleNamespace,
    }
    if name == "internal_state":
        class InternalState:
            call: Any
            agentic_loop: AgenticLoopState
            router: Any
            proxy: Any
            entrypoint: Any
        return InternalState
    try:
        return owners[name]
    except KeyError as error:
        raise TypeError(f"{name} is not a dataclass owner") from error


def projection(input_value: dict[str, Any]) -> dict[str, Any]:
    owner_type = owner(cast(str, input_value["owner"]))
    result: dict[str, Any] = {
        "wire": dict(wire(cast(str, input_value["name"]))),
        "names": list(wire_names(owner_type)),
    }
    if input_value.get("root") is not None:
        result["owned"] = list(owned_wire_names(owner(cast(str, input_value["root"]))))
    else:
        result["owned"] = None
    return result


def subset() -> dict[str, list[str]]:
    return {
        "agentic_loop": [*wire_names(AgenticLoopState), *wire_names(AgenticLoopOptions)],
        "bedrock_batch": list(wire_names(BedrockBatchConnection)),
    }


def run(input_value: dict[str, Any]) -> dict[str, Any]:
    function = cast(str, input_value["function"])
    if function == "wire_name_projection":
        return projection(input_value)
    if function == "owned_name_registry":
        return {
            "names": list(all_litellm_params),
            "owned": [
                is_litellm_owned_kwarg(name)
                for name in cast(list[str], input_value["names"])
            ],
        }
    if function == "internal_prefix_membership":
        return {
            "owned": [
                is_litellm_owned_kwarg(name)
                for name in cast(list[str], input_value["names"])
            ]
        }
    if function == "subset_registries":
        return subset()
    raise ValueError(f"unknown function: {function}")
