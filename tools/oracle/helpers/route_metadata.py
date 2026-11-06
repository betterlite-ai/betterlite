from typing import Literal, NotRequired, TypedDict, cast

from litellm.litellm_core_utils.api_route_to_call_types import (
    get_call_types_for_route,
    get_routes_for_call_type,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.types.utils import API_ROUTE_TO_CALL_TYPES, CallTypes


class Input(TypedDict):
    function: Literal["full_table", "get_call_types", "get_routes"]
    route: NotRequired[str]
    call_type: NotRequired[str]


class Route(TypedDict):
    route: str
    call_types: list[str]


class Catalog(TypedDict):
    source: str
    call_types: list[str]
    routes: list[Route]


def run(input_value: Input) -> object:
    function = input_value["function"]
    if function == "full_table":
        return Catalog(
            source="litellm@d665dc1a6315847fcad6cc2c1264ceb47c06abce",
            call_types=[item.value for item in CallTypes],
            routes=[
                Route(route=route, call_types=[item.value for item in values])
                for route, values in API_ROUTE_TO_CALL_TYPES.items()
            ],
        )
    if function == "get_call_types":
        route = input_value.get("route", "")
        result = get_call_types_for_route(route)
        if result is None:
            return {"call_types": None}
        return {"call_types": [item.value for item in result]}
    call_type_name = input_value.get("call_type", "")
    call_type = CallTypes(call_type_name)
    return cast(list[str], get_routes_for_call_type(call_type))
