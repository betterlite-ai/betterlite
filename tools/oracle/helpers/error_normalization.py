from collections.abc import Mapping

from litellm.exceptions import AuthenticationError, RateLimitError
from litellm.litellm_core_utils.error_normalization import normalize_error


class ProxyError(Exception):
    def __init__(self, message: str, proxy_type: str | None) -> None:
        super().__init__(message)
        if proxy_type is not None:
            self.type = proxy_type


def run(call: Mapping[str, object]) -> dict[str, str | None]:
    message_value = call.get("message")
    message = message_value if isinstance(message_value, str) else ""
    proxy_type_value = call.get("proxy_type")
    proxy_type = proxy_type_value if isinstance(proxy_type_value, str) else None
    status_value = call.get("status_code")
    status_code = str(status_value) if isinstance(status_value, int) else ""
    error_present = call.get("error_present") is not False
    class_value = call.get("class")
    error: Exception | None
    if not error_present:
        error = None
    elif class_value == "AuthenticationError":
        error = AuthenticationError(message, llm_provider="openai", model="model")
    elif class_value == "RateLimitError":
        type_value = call.get("rate_limit_type")
        error = RateLimitError(
            message, llm_provider="openai", model="model",
            rate_limit_type=type_value if isinstance(type_value, str) else None,
        )
    else:
        error = ProxyError(message, proxy_type)
    return {"result": normalize_error(error, status_code, message)}
