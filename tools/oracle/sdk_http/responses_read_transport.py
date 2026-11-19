import asyncio
from collections.abc import Callable, Coroutine
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Final, Literal, Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.responses.transformation import BaseResponsesAPIConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.llms.openai.responses.transformation import OpenAIResponsesAPIConfig
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter, ValidationError

Operation = Literal["retrieve", "input_items"]
Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
# the handlers take dict[str, object] maps, so the JSON-facing maps bind to that shape
OBJECTS: Final = TypeAdapter(dict[str, object] | None)
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException

RetrieveWork = ResponsesAPIResponse | Coroutine[object, object, ResponsesAPIResponse]
InputItemsWork = dict[str, object] | Coroutine[object, object, dict[str, object]]

RetrieveCall = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        dict[str, object] | None,
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    RetrieveWork,
]

AsyncRetrieveCall = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        dict[str, object] | None,
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    Coroutine[object, object, ResponsesAPIResponse],
]

InputItemsCall = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        str | None,
        str | None,
        list[str] | None,
        int,
        Literal["asc", "desc"],
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    InputItemsWork,
]

AsyncInputItemsCall = Callable[
    [
        str,
        BaseResponsesAPIConfig,
        GenericLiteLLMParams,
        Logging,
        str,
        str | None,
        str | None,
        list[str] | None,
        int,
        Literal["asc", "desc"],
        dict[str, object] | None,
        float | httpx.Timeout | None,
    ],
    Coroutine[object, object, dict[str, object]],
]


class Query(BaseModel):
    model_config = ConfigDict(extra="forbid")

    after: str | None = None
    before: str | None = None
    include: list[str] | None = None
    limit: int = 20
    order: Literal["asc", "desc"] = "desc"


class TransportRun(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    mode: Mode
    response_id: str
    query: Query = Field(default_factory=Query)
    extra_headers: dict[str, JsonValue] | None = None
    extra_body: dict[str, JsonValue] | None = None
    timeout_seconds: int | None = None


class TransportRuns(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[TransportRun]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int = Field(serialization_alias="status")
    status_code_is_synthesized: bool = Field(serialization_alias="synthesized")
    message: str
    headers: dict[str, str] | None = None


class Retrieved(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    created_at: int
    status: str | None
    model: str | None


class RunOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    retrieved: Retrieved | None = None
    result: JsonValue = None
    failure: Failure | None = None
    validation: bool = False


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[RunOutcome]


class TransportValue(BaseModel):
    value: Outcome


class TransportEnvelope(BaseModel):
    responses_read_transport: TransportValue


class Handlers(Protocol):
    @property
    def get_responses(self) -> RetrieveCall: ...

    @property
    def async_get_responses(self) -> AsyncRetrieveCall: ...

    @property
    def list_responses_input_items(self) -> InputItemsCall: ...

    @property
    def async_list_responses_input_items(  # ast-grep-ignore: no-long-names
        self,
    ) -> AsyncInputItemsCall: ...


HANDLER: Handlers = BaseLLMHTTPHandler()


@dataclass(frozen=True)
class Prepared:
    run: TransportRun
    config: BaseResponsesAPIConfig
    params: GenericLiteLLMParams
    logging_obj: Logging
    extra_headers: dict[str, object] | None
    extra_body: dict[str, object] | None
    timeout: float | None


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages="",
        stream=False,
        call_type="responses",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="responses-read-transport",
        function_id="responses-read-transport",
    )


def prepared_run(run: TransportRun, config: BaseResponsesAPIConfig, api_base: str) -> Prepared:
    return Prepared(
        run=run,
        config=config,
        params=GenericLiteLLMParams(api_base=api_base),
        logging_obj=logging_fixture(),
        extra_headers=OBJECTS.validate_python(run.extra_headers),
        extra_body=OBJECTS.validate_python(run.extra_body),
        timeout=None if run.timeout_seconds is None else float(run.timeout_seconds),
    )


def run_transport(run: TransportRun, api_base: str) -> RunOutcome:
    config: BaseResponsesAPIConfig = OpenAIResponsesAPIConfig()
    try:
        outcome = execute(run, config, api_base)
    except PROVIDER_ERROR as error:
        return RunOutcome(failure=Failure.model_validate(error))
    except ValidationError:
        return RunOutcome(validation=True)
    return outcome


def execute(run: TransportRun, config: BaseResponsesAPIConfig, api_base: str) -> RunOutcome:
    prepared = prepared_run(run, config, api_base)
    if run.operation == "retrieve":
        return RunOutcome(retrieved=retrieve(prepared))
    return RunOutcome(result=JSON.validate_python(input_items(prepared)))


def retrieve(prepared: Prepared) -> Retrieved:
    run = prepared.run
    if run.mode == "sync":
        return project_response(
            HANDLER.get_responses(
                run.response_id,
                prepared.config,
                prepared.params,
                prepared.logging_obj,
                "openai",
                prepared.extra_headers,
                prepared.extra_body,
                prepared.timeout,
            )
        )
    return project_response(
        asyncio.run(
            HANDLER.async_get_responses(
                run.response_id,
                prepared.config,
                prepared.params,
                prepared.logging_obj,
                "openai",
                prepared.extra_headers,
                prepared.extra_body,
                prepared.timeout,
            )
        )
    )


def input_items(prepared: Prepared) -> dict[str, object]:
    run = prepared.run
    if run.mode == "sync":
        value = HANDLER.list_responses_input_items(
            run.response_id,
            prepared.config,
            prepared.params,
            prepared.logging_obj,
            "openai",
            run.query.after,
            run.query.before,
            run.query.include,
            run.query.limit,
            run.query.order,
            prepared.extra_headers,
            prepared.timeout,
        )
    else:
        value = asyncio.run(
            HANDLER.async_list_responses_input_items(
                run.response_id,
                prepared.config,
                prepared.params,
                prepared.logging_obj,
                "openai",
                run.query.after,
                run.query.before,
                run.query.include,
                run.query.limit,
                run.query.order,
                prepared.extra_headers,
                prepared.timeout,
            )
        )
    if isinstance(value, Coroutine):
        raise TypeError("the sync handler returned a coroutine")
    return value


def project_response(value: RetrieveWork) -> Retrieved:
    if not isinstance(value, ResponsesAPIResponse):
        raise TypeError("the sync handler returned a coroutine")
    return Retrieved(
        id=value.id,
        created_at=value.created_at,
        status=value.status,
        model=value.model,
    )


class ResponsesReadTransportRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_read_transport: TransportRuns

    def serialize(self, api_base: str) -> str:
        runs = [run_transport(run, api_base) for run in self.responses_read_transport.runs]
        value = TransportValue(value=Outcome(runs=runs))
        return TransportEnvelope(responses_read_transport=value).model_dump_json(by_alias=True)
