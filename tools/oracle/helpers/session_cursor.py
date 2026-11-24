from typing import TypedDict

from litellm.proxy.spend_tracking import spend_management_endpoints
from pydantic import JsonValue


class CursorInput(TypedDict):
    cursor: str | None


def run(input_value: CursorInput) -> JsonValue:
    parsed = spend_management_endpoints._parse_session_cursor(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_management_endpoints.py#L3084
        input_value["cursor"]
    )
    if parsed is None:
        return None
    last_activity, session_key, api_key = parsed
    return [last_activity, session_key, api_key]
