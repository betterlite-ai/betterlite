from collections.abc import Mapping
from typing import Final

from litellm.proxy.common_utils.error_body_call_id import error_body_call_id

SETTING: Final[str] = "include_call_id_in_error_body"


def run(call: Mapping[str, object]) -> dict[str, object | None]:
    enabled = call.get("enabled") is True
    general_settings: dict[str, object] = {SETTING: enabled}
    call_id_value = call.get("call_id")
    call_id = call_id_value if isinstance(call_id_value, str) else None
    return {"result": error_body_call_id(general_settings, call_id)}
