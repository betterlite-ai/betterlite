from typing import Final, NoReturn

from pydantic import TypeAdapter
from typing_extensions import TypedDict

from betterlite._completion.errors import ERROR_PAYLOAD, ErrorPayload, raise_native_error
from betterlite._native import NativeAudioError
from betterlite.exceptions._openai_base import APIError


class ProviderBody(TypedDict):
    type: str | None
    param: str | None
    code: str | None


PROVIDER_BODY: Final[TypeAdapter[ProviderBody]] = TypeAdapter(ProviderBody)


# Local audio ValueErrors stay ValueErrors; every provider class keeps its own exception.
def raise_audio_error(error: NativeAudioError) -> NoReturn:
    payload: ErrorPayload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] == "ValueError":
        raise ValueError(payload.get("message")) from error
    try:
        raise_native_error(error)
    except APIError as mapped:
        body = payload.get("body")
        if mapped.body is None and body is not None:
            fields = PROVIDER_BODY.validate_python(body)
            mapped.body = body
            mapped.type = fields["type"]
            mapped.param = fields["param"]
            mapped.code = fields["code"]
        raise
