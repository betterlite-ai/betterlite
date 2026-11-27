import json
from collections.abc import Mapping
from typing import Final, NoReturn, NotRequired, TypedDict

from pydantic import TypeAdapter

from betterlite._completion.errors import raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeRealtimeError, NativeRealtimeSession
from betterlite.types.realtime import (
    RealtimeClientSecretResponse,
    RealtimeTranscriptionSessionResponse,
)

# the betterlite module globals the Rust session resolves its settings from
SETTINGS: Final[tuple[str, ...]] = (
    "api_base",
    "api_key",
    "api_version",
    "azure_key",
    "openai_key",
    "organization",
    "ssl_verify",
    "vertex_location",
    "vertex_project",
)


class ErrorPayload(TypedDict):
    type: str
    message: NotRequired[str]
    status_code: NotRequired[int]
    headers: NotRequired[dict[str, str]]


ERROR_PAYLOAD: Final[TypeAdapter[ErrorPayload]] = TypeAdapter(ErrorPayload)


# litellm/llms/base_llm/chat/transformation.py's BaseLLMException, raised by every realtime HTTP config
class BaseLLMException(Exception):
    status_code: int
    message: str
    headers: dict[str, str] | None

    def __init__(
        self, status_code: int, message: str, headers: dict[str, str] | None = None
    ) -> None:
        self.status_code = status_code
        self.message = message
        self.headers = headers
        super().__init__(message)


# litellm/llms/custom_httpx/http_handler.py's MaskedHTTPStatusError, the handler without a config's escape
class MaskedHTTPStatusError(Exception):
    status_code: int
    message: str
    headers: dict[str, str] | None

    def __init__(
        self, status_code: int, message: str, headers: dict[str, str] | None = None
    ) -> None:
        self.status_code = status_code
        self.message = message
        self.headers = headers
        super().__init__(message)


# The two realtime classes above are not in the shared table, so this family maps them and leaves the rest there.
def raise_realtime_error(error: NativeRealtimeError) -> NoReturn:
    payload = ERROR_PAYLOAD.validate_json(str(error))
    message = payload.get("message", "")
    headers = payload.get("headers")
    status_code = payload.get("status_code", 500)
    match payload["type"]:
        case "BaseLLMException":
            raise BaseLLMException(status_code, message, headers) from error
        case "MaskedHTTPStatusError":
            raise MaskedHTTPStatusError(status_code, message, headers) from error
        case "ValueError":
            raise ValueError(message) from error
        case object():
            raise_native_error(error)


def realtime_session() -> NativeRealtimeSession:
    return NativeRealtimeSession(
        json.dumps({"litellm_settings": read_settings(SETTINGS)})
    )


async def acreate_realtime_client_secret(  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L128-L183
    model: str | None = None,
    session: Mapping[str, CallValue] | None = None,
    expires_after: Mapping[str, CallValue] | None = None,
    timeout: float | None = None,
    **kwargs: CallValue,
) -> RealtimeClientSecretResponse:
    payload: CallPayload = to_call_payload(
        {
            "model": model,
            "session": session,
            "expires_after": expires_after,
            "timeout": timeout,
            **kwargs,
        }
    )
    try:
        answer = await realtime_session().aclient_secret(payload.text, payload.blobs)
    except NativeRealtimeError as failure:
        raise_realtime_error(failure)
    return RealtimeClientSecretResponse.model_validate_json(answer)


async def acreate_realtime_transcription_session(  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L189-L264
    model: str | None = None,
    transcription_session: Mapping[str, CallValue] | None = None,
    timeout: float | None = None,
    **kwargs: CallValue,
) -> RealtimeTranscriptionSessionResponse:
    payload: CallPayload = to_call_payload(
        {
            "model": model,
            "transcription_session": transcription_session,
            "timeout": timeout,
            **kwargs,
        }
    )
    try:
        answer = await realtime_session().atranscription_session(
            payload.text, payload.blobs
        )
    except NativeRealtimeError as failure:
        raise_realtime_error(failure)
    return RealtimeTranscriptionSessionResponse.model_validate_json(answer)


async def arealtime_calls(  # noqa: PLR0913, ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L270-L328
    openai_ephemeral_key: str,
    sdp_body: bytes,
    model: str | None = None,
    session: Mapping[str, CallValue] | None = None,
    timeout: float | None = None,
    **kwargs: CallValue,
) -> str:
    payload: CallPayload = to_call_payload(
        {
            "openai_ephemeral_key": openai_ephemeral_key,
            "model": model,
            "session": session,
            "timeout": timeout,
            **kwargs,
        }
    )
    try:
        return await realtime_session().acalls(
            payload.text, [*payload.blobs, sdp_body]
        )
    except NativeRealtimeError as failure:
        raise_realtime_error(failure)
