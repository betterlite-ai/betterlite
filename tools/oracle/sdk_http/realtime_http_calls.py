import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import BaseModel, ConfigDict, Field, JsonValue
from realtime_http_transport import (
    JSON,
    OBJECTS,
    PROVIDER_ERROR,
    TEXT,
    Answer,
    Failure,
    Strict,
    answer,
    masked_failure,
    provider_failure,
)

# an unknown model prints LiteLLM's provider list to stdout, where only the answer belongs
litellm.suppress_debug_info = True

Operation = Literal["client_secret", "transcription_session", "calls"]

AsyncResponse = Coroutine[object, object, httpx.Response]


# litellm's @wrapper_client decorator erases the wrapper signature, so the protocol names the three calls
class RealtimeCalls(Protocol):
    def acreate_realtime_client_secret(self, **kwargs: object) -> AsyncResponse: ...

    def acreate_realtime_transcription_session(  # ast-grep-ignore: no-long-names
        self, **kwargs: object
    ) -> AsyncResponse: ...

    def arealtime_calls(self, **kwargs: object) -> AsyncResponse: ...


CALLS: Final[RealtimeCalls] = cast(RealtimeCalls, litellm)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L127-L310


class Settings(Strict):
    api_base: str | None = None
    api_key: str | None = None
    openai_key: str | None = None


class CallsCall(Strict):
    operation: Operation
    settings: Settings = Field(default_factory=Settings)
    model: str | None = None
    api_key: str | None = None
    session: JsonValue = None
    expires_after: JsonValue = None
    transcription_session: JsonValue = None
    sdp: list[int] = Field(default_factory=list)
    ephemeral_key: str = ""
    extra_headers: dict[str, str] | None = None
    api_version: str | None = None


class Logged(Strict):
    model: str
    custom_llm_provider: str
    optional_params: JsonValue
    litellm_api_base: str
    session: JsonValue = None


class Outcome(Strict):
    result: Answer | None = None
    failure: Failure | None = None
    logged: Logged | None = None


class CallsValue(BaseModel):
    value: Outcome


class CallsEnvelope(BaseModel):
    realtime_http_calls: CallsValue


# The details the SDK's update_from_kwargs and the handler's pre_call leave behind
def logged(log: Logging) -> Logged | None:
    details = OBJECTS.validate_python(log.model_call_details)  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownArgumentType]  # LiteLLM types model_call_details as dict[str, Any]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L707
    if details is None:
        return None
    params = OBJECTS.validate_python(details.get("litellm_params"))
    args = OBJECTS.validate_python(details.get("additional_args"))
    if args is None or params is None:
        return None
    return Logged(
        model=TEXT.validate_python(details.get("model")),
        custom_llm_provider=TEXT.validate_python(details.get("custom_llm_provider")),
        optional_params=JSON.validate_python(details.get("optional_params")),
        litellm_api_base=TEXT.validate_python(params.get("api_base")),
        session=JSON.validate_python(args.get("session")),
    )


def called(call: CallsCall, api_base: str, log: Logging) -> AsyncResponse:
    litellm.api_base = call.settings.api_base
    litellm.api_key = call.settings.api_key
    litellm.openai_key = call.settings.openai_key
    forwarded: dict[str, object] = {
        "api_base": api_base,
        "api_key": call.api_key,
        "extra_headers": call.extra_headers,
        "api_version": call.api_version,
        "litellm_logging_obj": log,
    }
    if call.operation == "calls":
        return CALLS.arealtime_calls(
            **forwarded,
            openai_ephemeral_key=call.ephemeral_key,
            sdp_body=bytes(call.sdp),
            model=call.model,
            session=call.session,
        )
    if call.operation == "transcription_session":
        return CALLS.acreate_realtime_transcription_session(
            **forwarded,
            model=call.model,
            transcription_session=call.transcription_session,
        )
    return CALLS.acreate_realtime_client_secret(
        **forwarded,
        model=call.model,
        session=call.session,
        expires_after=call.expires_after,
    )


class RealtimeHttpCallsRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    realtime_http_calls: CallsCall

    def serialize(self, api_base: str) -> str:
        call = self.realtime_http_calls
        log = Logging(
            model="",
            messages="",
            stream=False,
            call_type="realtime_http_calls",
            start_time=datetime(2026, 1, 1, tzinfo=UTC),
            litellm_call_id="realtime-http-calls",
            function_id="realtime-http-calls",
        )
        try:
            response = asyncio.run(called(call, api_base, log))
        except PROVIDER_ERROR as error:
            outcome = Outcome(failure=provider_failure(error), logged=logged(log))
        except httpx.HTTPStatusError as error:
            outcome = Outcome(failure=masked_failure(error), logged=logged(log))
        except ValueError as error:
            failure = Failure(cls=type(error).__name__, message=str(error), headers={})
            outcome = Outcome(failure=failure, logged=logged(log))
        else:
            outcome = Outcome(result=answer(response), logged=logged(log))
        value = CallsValue(value=outcome)
        return CallsEnvelope(realtime_http_calls=value).model_dump_json(by_alias=True)
