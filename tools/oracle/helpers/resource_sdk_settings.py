from typing import TypedDict, cast


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
    if raw_timeout is not None and not isinstance(raw_timeout, str):
        raise ValueError("timeout must be a string or null")
    structured = input_value.get("structured")
    supports = provider in {"openai", "azure", "bedrock"}
    if structured is not None:
        if not isinstance(structured, dict):
            raise ValueError("structured must be an object or null")
        structured_value = cast(Structured, structured)
        timeout = structured_value if supports else structured_value["read"] or 600
    else:
        timeout = (
            float(raw_timeout.replace("_", ""))
            if raw_timeout not in (None, "", "0")
            else 600.0
        )
    request_timeout = input_value.get("request_timeout")
    global_timeout = input_value.get("global_request_timeout")
    numeric = (int, float, type(None))
    if not isinstance(request_timeout, numeric) or not isinstance(global_timeout, numeric):
        raise ValueError("request timeout values must be numbers or null")
    selected = float(request_timeout or global_timeout or 600)
    return {"supports_httpx_timeout": supports, "timeout": timeout, "request_timeout": selected}
