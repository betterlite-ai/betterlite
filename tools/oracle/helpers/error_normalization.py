from collections.abc import Mapping

from litellm import exceptions
from litellm.exceptions import RateLimitError
from litellm.litellm_core_utils.error_normalization import normalize_error

CLASSES: dict[str, type[Exception]] = {
    "AuthenticationError": exceptions.AuthenticationError,
    "PermissionDeniedError": exceptions.PermissionDeniedError,
    "ContextWindowExceededError": exceptions.ContextWindowExceededError,
    "ContentPolicyViolationError": exceptions.ContentPolicyViolationError,
    "GuardrailRaisedException": exceptions.GuardrailRaisedException,
    "BlockedPiiEntityError": exceptions.BlockedPiiEntityError,
    "UnsupportedParamsError": exceptions.UnsupportedParamsError,
    "NotFoundError": exceptions.NotFoundError,
    "Timeout": exceptions.Timeout,
    "MidStreamFallbackError": exceptions.MidStreamFallbackError,
    "APIConnectionError": exceptions.APIConnectionError,
    "ServiceUnavailableError": exceptions.ServiceUnavailableError,
    "InternalServerError": exceptions.InternalServerError,
    "BadGatewayError": exceptions.BadGatewayError,
    "BadRequestError": exceptions.BadRequestError,
    "UnprocessableEntityError": exceptions.UnprocessableEntityError,
    "BudgetExceededError": exceptions.BudgetExceededError,
    "NotImplementedError": NotImplementedError,
    "TypeError": TypeError,
    "KeyError": KeyError,
    "AttributeError": AttributeError,
    "IndexError": IndexError,
    "RuntimeError": RuntimeError,
    "AssertionError": AssertionError,
    "ZeroDivisionError": ZeroDivisionError,
}


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
    elif class_value == "RateLimitError":
        type_value = call.get("rate_limit_type")
        error = RateLimitError(
            message, llm_provider="openai", model="model",
            rate_limit_type=type_value if isinstance(type_value, str) else None,
        )
    else:
        error_type = (
            CLASSES.get(class_value, Exception) if isinstance(class_value, str) else Exception
        )
        error = Exception.__new__(error_type)
    if error is not None and proxy_type is not None:
        error.__dict__["type"] = proxy_type
    return {"result": normalize_error(error, status_code, message)}
