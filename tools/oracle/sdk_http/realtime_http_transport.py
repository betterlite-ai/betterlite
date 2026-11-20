import asyncio
from datetime import UTC, datetime
from typing import Final, Literal

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.realtime.http_transformation import BaseRealtimeHTTPConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

Handler = Literal["client_secrets", "transcription_sessions", "calls"]

HANDLER: Final = BaseLLMHTTPHandler()
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
REQUEST: Final = TypeAdapter(dict[str, object])
TEXT: Final = TypeAdapter(str)

# the echo provider names its own endpoints, so the URL proves which config member ran
REFUSED: Final = "the echo config refuses this model"


class EchoConfig(BaseRealtimeHTTPConfig):
    def get_api_base(self, api_base: str | None, **_kwargs: object) -> str:
        return api_base or ""

    def get_api_key(self, api_key: str | None, **_kwargs: object) -> str:
        return api_key or ""

    def get_complete_url(
        self,
        api_base: str | None,
        model: str,
        api_version: str | None = None,
    ) -> str:
        return (api_base or "") + "/custom/" + model + "/" + (api_version or "none")

    def get_transcription_session_url(
        self,
        api_base: str | None,
        model: str,
        api_version: str | None = None,
    ) -> str:
        return (api_base or "") + "/transcription/" + model + "/" + (api_version or "none")

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
    ) -> dict[str, str]:
        if model == "refuse":
            raise ValueError(REFUSED)
        return {**headers, "api-key": api_key or "", "Content-Type": "application/json"}

    def get_realtime_calls_url(
        self,
        api_base: str | None,
        model: str,
        api_version: str | None = None,
    ) -> str:
        return (api_base or "").rstrip("/") + "/calls/" + model + "/" + (api_version or "none")

    def get_realtime_calls_headers(self, ephemeral_key: str) -> dict[str, str]:
        return {"x-ephemeral": ephemeral_key}


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class TransportCall(Strict):
    handler: Handler
    echo_config: bool = False
    api_key: str = ""
    ephemeral_key: str | None = None
    request_data: dict[str, JsonValue] | None = None
    sdp: list[int] = Field(default_factory=list)
    session: dict[str, JsonValue] | None = None
    model: str | None = None
    extra_headers: dict[str, str] | None = None
    api_version: str | None = None
    timeout_seconds: int


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
    input: JsonValue
    api_base: str
    header_names: list[str]
    complete_input_dict: JsonValue = None
    session: JsonValue = None
    litellm_api_base: str


class Outcome(Strict):
    result: Answer | None = None
    failure: Failure | None = None
    logged: Logged | None = None


class TransportValue(BaseModel):
    value: Outcome


class TransportEnvelope(BaseModel):
    realtime_http_transport: TransportValue


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
        call_type="realtime_http_transport",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="realtime-http-transport",
        function_id="realtime-http-transport",
    )


def provider_failure(error: BaseLLMException) -> Failure:
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


def raw_failure(error: ValueError) -> Failure:
    return Failure(cls=type(error).__name__, message=str(error), headers={})


def answer(response: httpx.Response) -> Answer:
    return Answer(
        status=response.status_code,
        headers=dict(response.headers),
        body=list(response.content),
    )


# The shared redaction masks the header values betterlite records, so the projection compares names
def logged(log: Logging) -> Logged | None:
    details = OBJECTS.validate_python(log.model_call_details)
    if details is None:
        return None
    args = OBJECTS.validate_python(details.get("additional_args"))
    params = OBJECTS.validate_python(details.get("litellm_params"))
    if args is None or params is None:
        return None
    headers = OBJECTS.validate_python(args.get("headers"))
    if headers is None:
        return None
    return Logged(
        input=JSON.validate_python(details.get("input")),
        api_base=TEXT.validate_python(args.get("api_base")),
        header_names=sorted(headers),
        complete_input_dict=JSON.validate_python(args.get("complete_input_dict")),
        session=JSON.validate_python(args.get("session")),
        litellm_api_base=TEXT.validate_python(params.get("api_base")),
    )


async def post(call: TransportCall, api_base: str, log: Logging) -> httpx.Response:
    config = EchoConfig() if call.echo_config else None
    timeout = float(call.timeout_seconds)
    if call.handler == "calls":
        return await HANDLER.async_realtime_calls_handler(
            api_base=api_base,
            openai_ephemeral_key=TEXT.validate_python(call.ephemeral_key),
            sdp_body=bytes(call.sdp),
            logging_obj=log,
            timeout=timeout,
            provider_config=config,
            model=call.model,
            session_config=OBJECTS.validate_python(call.session),
            extra_headers=OBJECTS.validate_python(call.extra_headers),
            api_version=call.api_version,
        )
    if call.handler == "client_secrets":
        method = HANDLER.async_realtime_client_secret_handler
    else:
        method = HANDLER.async_realtime_transcription_session_handler
    return await method(
        api_base=api_base,
        api_key=call.api_key,
        request_data=REQUEST.validate_python(call.request_data),
        logging_obj=log,
        timeout=timeout,
        provider_config=config,
        model=call.model,
        extra_headers=OBJECTS.validate_python(call.extra_headers),
        api_version=call.api_version,
    )


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    log = logging_fixture()
    try:
        response = asyncio.run(post(call, api_base, log))
    except BaseLLMException as error:
        return Outcome(failure=provider_failure(error), logged=logged(log))
    except httpx.HTTPStatusError as error:
        return Outcome(failure=masked_failure(error), logged=logged(log))
    except ValueError as error:
        return Outcome(failure=raw_failure(error), logged=logged(log))
    return Outcome(result=answer(response), logged=logged(log))


class RealtimeHttpTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    realtime_http_transport: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.realtime_http_transport, api_base)
        value = TransportValue(value=outcome)
        return TransportEnvelope(realtime_http_transport=value).model_dump_json(by_alias=True)
