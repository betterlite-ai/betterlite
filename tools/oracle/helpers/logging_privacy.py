from __future__ import annotations

from typing import TypeAlias

import litellm

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
REDACTED = "[REDACTED]"


def boolish(value: Json) -> bool | None:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        lowered = value.lower()
        if lowered in {"true", "1", "yes", "on"}:
            return True
        if lowered in {"false", "0", "no", "off"}:
            return False
    return None


def redact(value: Json) -> Json:
    if isinstance(value, list):
        return [redact(item) for item in value]
    if not isinstance(value, dict):
        return value
    result: dict[str, Json] = {}
    for key, child in value.items():
        if key == "messages":
            result[key] = [{"role": "user", "content": REDACTED}]
        elif key in {"prompt", "input"}:
            result[key] = ""
        elif (
            key in {"content", "reasoning_content", "text", "refusal", "arguments"}
            and child is not None
        ):
            result[key] = REDACTED
        elif key in {"thinking_blocks", "audio"}:
            result[key] = None
        else:
            result[key] = redact(child)
    return result


def run(value: Json) -> Json:
    if not isinstance(value, dict):
        raise ValueError("input must be an object")
    # Keep the import explicit: this adapter is checked against the pinned LiteLLM package.
    _ = litellm.turn_off_message_logging
    headers = value.get("headers")
    disabled = isinstance(headers, dict) and (
        boolish(headers.get("litellm-disable-message-redaction")) is True
    )
    enabled = isinstance(headers, dict) and any(
        boolish(headers.get(name)) is True
        for name in ("litellm-enable-message-redaction", "x-litellm-enable-message-redaction")
    )
    dynamic = boolish(value.get("dynamic"))
    should_redact = False if disabled else dynamic
    if should_redact is None:
        should_redact = True if enabled else None
    if should_redact is None:
        should_redact = value.get("global") is True
    key_info = value.get("key_info")
    if value.get("redact_key_info") is True and isinstance(key_info, dict):
        key_info = {
            key: item for key, item in key_info.items() if not key.startswith("user_api_key")
        }
    payload = redact(value.get("payload")) if should_redact else value.get("payload")
    return {"should_redact": should_redact, "snapshot": payload, "key_info": key_info}
