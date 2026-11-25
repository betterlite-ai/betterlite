from typing import NoReturn

from betterlite._completion.errors import ERROR_PAYLOAD, ErrorPayload, raise_native_error
from betterlite._native import NativeAudioError


# Local audio ValueErrors stay ValueErrors; every provider class keeps its own exception.
def raise_audio_error(error: NativeAudioError) -> NoReturn:
    payload: ErrorPayload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] == "ValueError":
        raise ValueError(payload.get("message")) from error
    raise_native_error(error)
