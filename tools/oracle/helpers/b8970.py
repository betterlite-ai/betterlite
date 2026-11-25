from typing import TypedDict

from litellm.proxy._experimental.mcp_server.utils import MCP_TOOL_PREFIX_FORMAT
from mcp.shared.tool_name_validation import validate_tool_name


class Input(TypedDict):
    separator: str


def run(input_value: Input) -> bool:
    probe = MCP_TOOL_PREFIX_FORMAT.format(
        server_name="litellm", separator=input_value["separator"], tool_name="probe"
    )
    return validate_tool_name(probe).is_valid
