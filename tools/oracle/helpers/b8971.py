from typing import NamedTuple, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server import server as mcp_server

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class RequestContext(NamedTuple):
    meta: dict[str, Json] | None


class Input(TypedDict):
    meta: dict[str, Json] | None


def run(input_value: Input) -> dict[str, str] | None:
    return mcp_server._mcp_meta_trace_carrier(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        RequestContext(meta=input_value["meta"])
    )
