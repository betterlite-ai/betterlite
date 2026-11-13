from collections.abc import Callable
from typing import Protocol, TypeAlias, cast

from litellm.proxy.agent_endpoints import a2a_endpoints

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CompatModel(Protocol):
    def model_dump(
        self, *, by_alias: bool, exclude_none: bool, mode: str
    ) -> dict[str, Json]: ...


# the route reads the params through a private function, so the helper reaches it the same way
_ENDPOINTS = cast(dict[str, object], vars(a2a_endpoints))
_BUILD = cast(
    Callable[[dict[str, Json]], CompatModel], _ENDPOINTS["_build_message_send_params"]
)


def run(input_value: dict[str, Json]) -> Json:
    params = cast(dict[str, Json], input_value.get("params", {}))
    dumped = _BUILD(params).model_dump(by_alias=True, exclude_none=True, mode="json")
    return cast(Json, dumped)
