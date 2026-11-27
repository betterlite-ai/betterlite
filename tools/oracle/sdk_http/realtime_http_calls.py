import asyncio
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Literal, Protocol, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

# an unknown model prints LiteLLM's provider list to stdout, where only the answer belongs
litellm.suppress_debug_info = True

Operation = Literal["client_secret", "transcription_session", "calls"]

PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
TEXT: TypeAdapter[str] = TypeAdapter(str)

AsyncResponse = Coroutine[object, object, httpx.Response]


# litellm's @wrapper_client decorator erases the wrapper signature, so the protocol names the three calls
class RealtimeCalls(Protocol):
    def acreate_realtime_client_secret(
        self,
        *,
        model: str | None,
        session: JsonValue,
        expires_after: JsonValue,
        api_base: str,
        api_key: str | None,
        extra_headers: dict[str, str] | None,
        api_version: str | None,
        litellm_logging_obj: Logging,
    ) -> AsyncResponse: ...

    def acreate_realtime_transcription_session(
        self,
        *,
        model: str | None,
        transcription_session: JsonValue,
        api_base: str,
        api_key: str | None,
        extra_headers: dict[str, str] | None,
        api_version: str | None,
        litellm_logging_obj: Logging,
    ) -> AsyncResponse: ...

    def arealtime_calls(
        self,
        *,
        openai_ephemeral_key: str,
        sdp_body: bytes,
        model: str | None,
        session: JsonValue,
        api_base: str,
        api_key: str | None,
        extra_headers: dict[str, str] | None,
        api_version: str | None,
        litellm_logging_obj: Logging,
    ) -> AsyncResponse: ...


CALLS: Final[RealtimeCalls] = cast(RealtimeCalls, litellm)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/realtime_api/main.py#L127-L310


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


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


class Answer(Strict):
    status: int
    headers: dict[str, str]
    body: list[int]


class Failure(Strict):
    cls: str = Field(serialization_alias="class")
    status: int | None = None
    message: str
    headers: dict[str, str]
    synthesized: bool | None = None


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


# BaseLLMException carries its four observable fields as attributes, whatever their declared types
class ProviderError(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int
    status_code_is_synthesized: bool
    message: str
    headers: dict[str, str]


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="realtime_http_calls",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="realtime-http-calls",
        function_id="realtime-http-calls",
    )


def provider_failure(error: Exception) -> Failure:
    details = ProviderError.model_validate(error)
    return Failure(
        cls=type(error).__name__,
        status=details.status_code,
        message=details.message,
        headers=details.headers,
        synthesized=details.status_code_is_synthesized,
    )


def masked_failure(error: httpx.HTTPStatusError) -> Failure:
    return Failure(
        cls=type(error).__name__,
        status=error.response.status_code,
        message=error.response.text,
        headers=dict(error.response.headers),
    )


def answer(response: httpx.Response) -> Answer:
    return Answer(
        status=response.status_code,
        headers=dict(response.headers),
        body=list(response.content),
    )


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
    if call.operation == "calls":
        return CALLS.arealtime_calls(
            openai_ephemeral_key=call.ephemeral_key,
            sdp_body=bytes(call.sdp),
            model=call.model,
            session=call.session,
            api_base=api_base,
            api_key=call.api_key,
            extra_headers=call.extra_headers,
            api_version=call.api_version,
            litellm_logging_obj=log,
        )
    if call.operation == "transcription_session":
        return CALLS.acreate_realtime_transcription_session(
            model=call.model,
            transcription_session=call.transcription_session,
            api_base=api_base,
            api_key=call.api_key,
            extra_headers=call.extra_headers,
            api_version=call.api_version,
            litellm_logging_obj=log,
        )
    return CALLS.acreate_realtime_client_secret(
        model=call.model,
        session=call.session,
        expires_after=call.expires_after,
        api_base=api_base,
        api_key=call.api_key,
        extra_headers=call.extra_headers,
        api_version=call.api_version,
        litellm_logging_obj=log,
    )


def run_calls(call: CallsCall, api_base: str) -> Outcome:
    log = logging_fixture()
    try:
        with asyncio.Runner() as runner:
            response = runner.run(called(call, api_base, log))
    except PROVIDER_ERROR as error:
        return Outcome(failure=provider_failure(error), logged=logged(log))
    except httpx.HTTPStatusError as error:
        return Outcome(failure=masked_failure(error), logged=logged(log))
    except ValueError as error:
        failure = Failure(cls=type(error).__name__, message=str(error), headers={})
        return Outcome(failure=failure, logged=logged(log))
    return Outcome(result=answer(response), logged=logged(log))


class RealtimeHttpCallsRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    realtime_http_calls: CallsCall

    def serialize(self, api_base: str) -> str:
        outcome = run_calls(self.realtime_http_calls, api_base)
        value = CallsValue(value=outcome)
        return CallsEnvelope(realtime_http_calls=value).model_dump_json(by_alias=True)
