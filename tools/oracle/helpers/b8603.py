from typing import TypeAlias, TypedDict

from litellm.proxy.proxy_server import (
    _is_explicit_keepalive_disable,  # pyright: ignore[reportPrivateUsage]
)

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class Input(TypedDict):
    values: list[JsonValue]


def run(input_value: Input) -> list[bool]:
    return [_is_explicit_keepalive_disable(value) for value in input_value["values"]]
