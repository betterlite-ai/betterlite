from collections.abc import Mapping
from typing import TypeAlias, cast

from litellm.router_utils import add_retry_fallback_headers as headers

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    request = input_value.get("request")
    if function == "complexity":
        output: dict[str, Json] = dict(headers.complexity_router_decision_headers(request))
        return output
    if function == "replace":
        existing = input_value.get("existing")
        new_headers = input_value.get("new_headers")
        if not isinstance(existing, dict) or not isinstance(new_headers, dict):
            raise ValueError("replace inputs must be objects")
        return dict(
            cast(
                Mapping[str, Json], headers.replace_complexity_router_headers(existing, new_headers)
            )
        )
    if function == "quality":
        values = input_value.get("headers")
        if not isinstance(values, dict):
            raise ValueError("quality headers must be an object")
        additional_headers: dict[str, object] = dict(values)
        headers.apply_quality_router_decision_headers(additional_headers, request)
        return cast(Json, additional_headers)
    raise ValueError(f"unknown routing header function: {function}")
