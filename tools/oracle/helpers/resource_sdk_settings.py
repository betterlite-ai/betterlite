from typing import TypedDict, cast

import httpx
from litellm.fine_tuning.main import _resolve_fine_tuning_timeout  # pyright: ignore[reportPrivateUsage]
from litellm.utils import supports_httpx_timeout


class Structured(TypedDict):
    connect: float | None
    read: float | None
    write: float | None
    pool: float | None


def run(input_value: dict[str, object]) -> dict[str, object]:
    provider = input_value["provider"]
    if not isinstance(provider, str):
        raise ValueError("provider must be a string")
    raw_timeout = input_value.get("timeout")
    if raw_timeout is not None and not isinstance(raw_timeout, (int, float, str)):
        raise ValueError("timeout must be a number, string, or null")
    structured = input_value.get("structured")
    supports = supports_httpx_timeout(provider)
    if structured is not None:
        if not isinstance(structured, dict):
            raise ValueError("structured must be an object or null")
        structured_value = cast(Structured, structured)
        timeout = httpx.Timeout(**structured_value)
    else:
        timeout = raw_timeout
    resolved = _resolve_fine_tuning_timeout(timeout, provider)
    if isinstance(resolved, httpx.Timeout):
        timeout_result: object = {
            "connect": resolved.connect,
            "read": resolved.read,
            "write": resolved.write,
            "pool": resolved.pool,
        }
    else:
        timeout_result = resolved
    request_timeout = input_value.get("request_timeout")
    global_timeout = input_value.get("global_request_timeout")
    numeric = (int, float, type(None))
    if not isinstance(request_timeout, numeric) or not isinstance(global_timeout, numeric):
        raise ValueError("request timeout values must be numbers or null")
    selected = float(request_timeout or global_timeout or 600)
    return {
        "supports_httpx_timeout": supports,
        "timeout": timeout_result,
        "request_timeout": selected,
    }
