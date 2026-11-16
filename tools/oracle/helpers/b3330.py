from typing import Final, Literal, TypedDict

from litellm.proxy.video_endpoints.utils import (
    extract_model_from_target_model_names,
    get_custom_provider_from_data,
    video_reference_to_id,
)
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict):
    function: Literal["extract_model", "video_reference", "custom_provider"]
    value: JsonValue


DATA: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    value = input_value["value"]
    if function == "extract_model":
        return extract_model_from_target_model_names(value)
    if function == "video_reference":
        return video_reference_to_id(value)
    return get_custom_provider_from_data(DATA.validate_python(value))
