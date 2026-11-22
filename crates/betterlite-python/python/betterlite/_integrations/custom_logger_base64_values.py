import logging
import re
from typing import Final

from pydantic import JsonValue

from betterlite._integrations.custom_logger_types import SENSITIVE_DATA_MAX_DEPTH

LOGGER: Final = logging.getLogger("LiteLLM")
BASE64_INLINE_PATTERN: Final = re.compile(
    r"data:(?:application|image|audio|video)/[a-zA-Z0-9.+-]+;base64,[A-Za-z0-9+/=\s]+",
    re.MULTILINE,
)
BASE64_REDACTED: Final = "[BASE64_REDACTED]"
MAX_DEPTH_REACHED: Final = "[MAX_DEPTH_REACHED]"


class CustomLoggerBase64Values:
    def _redact_base64(
        self, value: JsonValue, depth: int = 0, max_depth: int = SENSITIVE_DATA_MAX_DEPTH
    ) -> JsonValue:
        if depth > max_depth:
            LOGGER.warning(
                "[CustomLogger] Max recursion depth %s reached while redacting base64", max_depth
            )
            return MAX_DEPTH_REACHED
        match value:
            case str():
                if BASE64_INLINE_PATTERN.search(value) is None:
                    return value
                LOGGER.debug("[CustomLogger] Redacted inline base64 string: %s...", value[:40])
                return BASE64_INLINE_PATTERN.sub(BASE64_REDACTED, value)
            case dict():
                mapping: dict[str, JsonValue] = value
                return {
                    key: self._redact_base64(value=item, depth=depth + 1, max_depth=max_depth)
                    for key, item in mapping.items()
                }
            case list():
                items: list[JsonValue] = value
                return [
                    self._redact_base64(value=item, depth=depth + 1, max_depth=max_depth)
                    for item in items
                ]
            case bool() | int() | float() | None:
                return value

    def _should_keep_content(self, content: JsonValue) -> bool:
        match content:
            case dict():
                content_payload: dict[str, JsonValue] = content
                if "file" in content_payload:
                    return False
                match content_payload.get("type"):
                    case str() as kind:
                        return kind == "text"
                    case bool() | int() | float() | list() | dict() | None:
                        return True
            case str() | bool() | int() | float() | list() | None:
                return True
