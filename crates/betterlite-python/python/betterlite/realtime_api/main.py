import json
from collections.abc import Mapping
from typing import Final, NoReturn, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter

from betterlite._completion.errors import raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, to_call_payload
from betterlite._endpoints.runner import AsyncNativeCall
from betterlite._endpoints.settings import read_settings
from betterlite._native import NativeRealtimeError, NativeRealtimeSession
from betterlite.types.realtime import (
    RealtimeClientSecretResponse,
    RealtimeTranscriptionSessionResponse as TranscriptionResponse,
)

# the betterlite module globals the Rust session resolves its settings from
SETTINGS: Final[tuple[str, ...]] = (
    "api_base",
    "api_key",
    "api_version",
    "azure_key",
    "openai_key",
    "organization",
    "vertex_location",
    "vertex_project",
)


class Failure(TypedDict):
    type: str
    message: str


class HttpFailure(Failure):
    status_code: int
    headers: NotRequired[dict[str, str]]


FAILURE: Final[TypeAdapter[Failure]] = TypeAdapter(Failure)
HTTP_FAILURE: Final[TypeAdapter[HttpFailure]] = TypeAdapter(HttpFailure)


# LiteLLM's BaseLLMException and MaskedHTTPStatusError, the class of every realtime HTTP failure
class RealtimeHttpFailure(Exception):
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


class BaseLLMException(RealtimeHttpFailure):  # noqa: N818  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41
    ...


class MaskedHTTPStatusError(RealtimeHttpFailure):
    ...


# Both classes are missing from the shared table; every other type stays with raise_native_error.
def raise_realtime_error(error: NativeRealtimeError) -> NoReturn:
    failure = FAILURE.validate_json(str(error))
    match failure["type"]:
        case "BaseLLMException":
            http = HTTP_FAILURE.validate_json(str(error))
            raise BaseLLMException(
                http["status_code"], http["message"], http.get("headers")
            ) from error
        case "MaskedHTTPStatusError":
            http = HTTP_FAILURE.validate_json(str(error))
            raise MaskedHTTPStatusError(
                http["status_code"], http["message"], http.get("headers")
            ) from error
        case "ValueError":
            raise ValueError(failure["message"]) from error
        case object():
            raise_native_error(error)


def realtime_session() -> NativeRealtimeSession:
    settings: dict[str, JsonValue] = {"litellm_settings": read_settings(SETTINGS)}
    return NativeRealtimeSession(json.dumps(settings))


async def realtime_answer(method: AsyncNativeCall, payload: CallPayload) -> str:
    try:
        return await method(payload.text, payload.blobs)
    except NativeRealtimeError as failure:
        raise_realtime_error(failure)


async def acreate_realtime_client_secret(  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L128-L183
    model: str | None = None,
    session: Mapping[str, CallValue] | None = None,
    expires_after: Mapping[str, CallValue] | None = None,
    timeout: float | None = None,
    **kwargs: CallValue,
) -> RealtimeClientSecretResponse:
    values: dict[str, CallValue] = {"model": model, "timeout": timeout, **kwargs}
    if session is not None:
        values["session"] = session
    if expires_after is not None:
        values["expires_after"] = expires_after
    answer = await realtime_answer(realtime_session().aclient_secret, to_call_payload(values))
    return RealtimeClientSecretResponse.model_validate_json(answer)


# ast-grep-ignore: no-long-names
async def acreate_realtime_transcription_session(  # noqa: ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L189-L264
    model: str | None = None,
    transcription_session: Mapping[str, CallValue] | None = None,
    timeout: float | None = None,
    **kwargs: CallValue,
) -> TranscriptionResponse:
    values: dict[str, CallValue] = {
        "model": model,
        "transcription_session": transcription_session,
        "timeout": timeout,
        **kwargs,
    }
    return TranscriptionResponse.model_validate_json(
        await realtime_answer(realtime_session().atranscription_session, to_call_payload(values))
    )


async def arealtime_calls(  # noqa: PLR0913, ASYNC109  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L270-L328
    openai_ephemeral_key: str,
    sdp_body: bytes,
    model: str | None = None,
    session: Mapping[str, CallValue] | None = None,
    timeout: float | None = None,
    **kwargs: CallValue,
) -> str:
    payload = to_call_payload(
        {
            "openai_ephemeral_key": openai_ephemeral_key,
            "model": model,
            "session": session,
            "timeout": timeout,
            **kwargs,
        }
    )
    offer = CallPayload(text=payload.text, blobs=[*payload.blobs, sdp_body])
    return await realtime_answer(realtime_session().acalls, offer)
