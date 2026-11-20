from typing import TypedDict

from litellm.proxy.management_helpers.object_permission_utils import _mcp_tool_permission_entries  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L324-L332
from pydantic import JsonValue


class Input(TypedDict):
    raw: JsonValue


def run(input_value: Input) -> dict[str, list[str]]:
    entries = _mcp_tool_permission_entries(input_value["raw"])
    return {identifier: sorted(tools) for identifier, tools in entries.items()}
