from typing import Final, Literal, TypeAlias, TypedDict

from litellm.litellm_core_utils.core_helpers import (
    bind_budget_reservation_to_callbacks,
    budget_reservation_from_metadata,
    unbind_budget_reservation_from_callbacks,
)
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class Lookup(TypedDict):
    function: Literal["lookup"]
    metadata: dict[str, JsonValue]


class Stamp(TypedDict):
    function: Literal["stamp"]
    litellm_params: dict[str, JsonValue]
    callback_bound: bool


Input: TypeAlias = Lookup | Stamp


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "lookup":
        found = budget_reservation_from_metadata(input_value["metadata"])
        return JSON.validate_python(found)
    if input_value["callback_bound"]:
        bind_budget_reservation_to_callbacks(input_value["litellm_params"])
    else:
        unbind_budget_reservation_from_callbacks(input_value["litellm_params"])
    return input_value["litellm_params"]
