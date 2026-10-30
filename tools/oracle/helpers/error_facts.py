import json
from typing import TypeAlias, cast

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]
RATE_LIMIT_STATUS = 429
BAD_REQUEST_STATUS = 400
SERVER_ERROR_STATUS = 500


def _string(argument: dict[str, JsonValue], key: str) -> str:
    value = argument.get(key)
    return value if isinstance(value, str) else ""


def _integer(argument: dict[str, JsonValue], key: str) -> int | None:
    value = argument.get(key)
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _boolean(argument: dict[str, JsonValue], key: str) -> bool:
    value = argument.get(key)
    return value if isinstance(value, bool) else False


def _rate_limit(argument: dict[str, JsonValue]) -> bool:
    text = _string(argument, "text")
    status = _integer(argument, "status")
    standalone = "429" in text.split()
    return bool(
        (standalone and (status is None or status == RATE_LIMIT_STATUS))
        or "rate limit" in text.lower()
        or "rate-limit" in text.lower()
        or "rate_limit" in text.lower()
        or "service tier capacity exceeded" in text.lower()
    )


def _context_window(argument: dict[str, JsonValue]) -> bool:
    text = _string(argument, "text").lower()
    if "string_above_max_length" in text or (
        "invalid 'user'" in text and "string too long" in text
    ):
        return False
    return any(
        phrase in text
        for phrase in (
            "exceed context limit",
            "this model's maximum context length is",
            "string too long. expected a string with maximum length",
            "model's maximum context limit",
            "is longer than the model's context length",
            "input tokens exceed the configured limit",
            "`inputs` tokens + `max_new_tokens` must be",
            "exceeds the available context size",
            "exceeds the maximum number of tokens allowed",
            "current length is",
            "maximum input length is",
        )
    )


def _body_code(argument: dict[str, JsonValue]) -> int | None:
    try:
        body = cast(dict[str, JsonValue], json.loads(_string(argument, "text")))
        error_value = body.get("error")
        if not isinstance(error_value, dict):
            return None
        code = error_value.get("code")
        if isinstance(code, bool) or not isinstance(code, (int, str)):
            return None
        return int(code)
    except (TypeError, ValueError, AttributeError):
        return None


def _message(argument: dict[str, JsonValue]) -> str | None:
    body_value = argument.get("body")
    body = cast(dict[str, JsonValue], body_value) if isinstance(body_value, dict) else None
    if not isinstance(body, dict):
        return None
    message = body.get("message")
    if message:
        return message if isinstance(message, str) else None
    nested_value = body.get("error")
    nested = cast(dict[str, JsonValue], nested_value) if isinstance(nested_value, dict) else None
    if nested is not None and isinstance(nested.get("message"), str):
        message_value = nested.get("message")
        return message_value if isinstance(message_value, str) else None
    return None


def _status(argument: dict[str, JsonValue]) -> dict[str, JsonValue] | None:
    status = _integer(argument, "status")
    if (
        not isinstance(status, int)
        or status < BAD_REQUEST_STATUS
        or _boolean(argument, "status_code_is_synthesized")
    ):
        return None
    family = {
        401: "authentication", 403: "permission_denied", 404: "not_found",
        408: "timeout", 429: "rate_limit", 500: "internal_server",
        502: "bad_gateway", 503: "service_unavailable", 504: "timeout",
    }.get(status, "bad_request" if status < SERVER_ERROR_STATUS else "api")
    return {
        "family": family,
        "status_code": status,
        "message": f"{_string(argument, 'provider') or 'Provider'} - {_string(argument, 'text')}",
        "response_present": bool(
            argument.get("body") is not None or argument.get("response_headers") is not None
        ),
    }


def run(argument: JsonValue) -> JsonValue:
    value = cast(dict[str, JsonValue], argument) if isinstance(argument, dict) else {}
    kind = value.get("kind")
    if kind == "rate_limit":
        return _rate_limit(value)
    if kind == "context_window":
        return _context_window(value)
    if kind == "body_code":
        return _body_code(value)
    if kind == "message":
        return _message(value)
    if kind == "status":
        return _status(value)
    return None
