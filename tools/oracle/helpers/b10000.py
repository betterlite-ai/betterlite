from types import SimpleNamespace
from typing import NotRequired, TypedDict

from litellm.proxy.management_helpers.object_permission_utils import _mcp_server_identifier_matches  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L251-L257


class Server(TypedDict):
    server_id: NotRequired[str]
    alias: NotRequired[str]
    server_name: NotRequired[str]
    name: NotRequired[str]


class Input(TypedDict):
    server: Server
    identifier: str


def run(input_value: Input) -> dict[str, bool]:
    server = SimpleNamespace(**input_value["server"])
    matches = _mcp_server_identifier_matches(server, input_value["identifier"])
    return {"matches": matches}
