from typing import NoReturn

from betterlite._completion.errors import ERROR_PAYLOAD, ErrorPayload, raise_native_error
from betterlite._native import NativeImageError


# image_variation raises a bare ValueError, which its module never maps to a LiteLLM class
def raise_image_error(error: NativeImageError) -> NoReturn:
    payload: ErrorPayload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] == "ValueError":
        raise ValueError(payload.get("message") or "") from error
    raise_native_error(error)
