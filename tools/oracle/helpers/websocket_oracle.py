import json
from typing import TypeAlias, cast

from litellm.litellm_core_utils.realtime_errors import (
    realtime_error_event,
    upstream_handshake_close_code,
    websocket_close_reason,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    if set(input_value) != {"status"}:
        raise ValueError("websocket input must contain status")
    status = input_value["status"]
    if not isinstance(status, int) or isinstance(status, bool):
        raise ValueError("status must be an integer")
    message = f"Upstream realtime handshake rejected with HTTP {status}"
    event = cast(
        dict[str, Json],
        json.loads(realtime_error_event(message, error_type="server_error")),
    )
    return {
        "event": event,
        "close_code": upstream_handshake_close_code(status),
        "close_reason": websocket_close_reason(
            message, fallback="Upstream handshake rejected"
        ),
    }
