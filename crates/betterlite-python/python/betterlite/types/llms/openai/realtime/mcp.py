from typing import Literal

from typing_extensions import Required, TypedDict


class OpenAIMcpServerTool(TypedDict, total=False):
    type: Required[Literal["mcp"]]
    server_label: Required[str]
    server_url: Required[str]
    require_approval: str
    allowed_tools: list[str] | None
    headers: dict[str, str] | None
