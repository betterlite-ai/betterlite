import asyncio
from collections.abc import Coroutine, Mapping
from datetime import UTC, datetime
from typing import Literal, TypeAlias

import httpx
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.rerank.transformation import BaseRerankConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.rerank import RerankResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import TypedDict, override

Mode = Literal["sync", "async"]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

RerankWork: TypeAlias = RerankResponse | Coroutine[object, object, RerankResponse]


class TransportCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    modes: list[Mode]
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    api_key: str | None = None
    timeout_seconds: int | None = None


class Failure(BaseModel):
    model_config = ConfigDict(extra="forbid")

    class_name: str = Field(serialization_alias="class")
    status: int
    kind: str | None = Field(default=None, serialization_alias="type")
    param: str | None = None
    code: str | None = None
    message: str


class PreCallSeen(BaseModel):
    model_config = ConfigDict(extra="forbid")

    input: JsonValue
    body: JsonValue


class TransformSeen(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    api_key: str | None
    request_data: JsonValue
    optional_params: JsonValue
    litellm_params: JsonValue
    raw_status: int
    raw_body: JsonValue


class RunOutcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    failure: Failure | None = None
    events: list[str]
    pre_call: PreCallSeen | None = None
    transform: TransformSeen | None = None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[RunOutcome]


class TransportValue(BaseModel):
    value: Outcome


class TransportEnvelope(BaseModel):
    rerank_http: TransportValue


class AdditionalArgs(TypedDict):
    complete_input_dict: JsonValue


class PreCallDetails(TypedDict):
    input: JsonValue
    additional_args: AdditionalArgs


PRECALL: TypeAdapter[PreCallDetails] = TypeAdapter(PreCallDetails)


class RecordingConfig(BaseRerankConfig):
    def __init__(self) -> None:
        self.events: list[str] = []
        self.seen: TransformSeen | None = None

    @override
    def validate_environment(
        self,
        headers: dict[str, JsonValue],
        model: str,
        api_key: str | None = None,
        optional_params: dict[str, JsonValue] | None = None,
        # ast-grep-ignore: no-loose-types
        litellm_params: Mapping[str, object] | None = None,
    ) -> dict[str, JsonValue]:
        self.events.append("validate")
        supplied: dict[str, JsonValue] = {}
        if api_key is not None:
            supplied["x-api-key"] = api_key
        return supplied

    @override
    def get_complete_url(
        self,
        api_base: str | None,
        model: str,
        optional_params: dict[str, JsonValue] | None = None,
    ) -> str:
        self.events.append("url")
        if api_base is None:
            raise ValueError("the case points the call at its mock")
        return api_base

    @override
    def transform_rerank_request(
        self,
        model: str,
        optional_rerank_params: dict[str, JsonValue],
        headers: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue] | None = None,
    ) -> dict[str, JsonValue]:
        self.events.append("transform")
        documents: JsonValue = (
            optional_rerank_params["documents"] if "documents" in optional_rerank_params else []
        )
        query: JsonValue = (
            optional_rerank_params["query"] if "query" in optional_rerank_params else ""
        )
        return {"documents": documents, "query": query}

    @override
    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: Logging,
        api_key: str | None = None,
        request_data: dict[str, JsonValue] | None = None,
        optional_params: dict[str, JsonValue] | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
    ) -> RerankResponse:
        self.events.append("response_transform")
        optional: dict[str, JsonValue] = (
            optional_params if optional_params is not None else {}
        )
        passed: dict[str, JsonValue] = litellm_params if litellm_params is not None else {}
        self.seen = TransformSeen(
            model=model,
            api_key=api_key,
            request_data=JSON.validate_python(request_data),
            optional_params=optional,
            litellm_params=passed,
            raw_status=raw_response.status_code,
            raw_body=JSON.validate_json(raw_response.text),
        )
        return RerankResponse.model_validate_json(raw_response.text)

    @override
    # ast-grep-ignore: no-long-names
    def get_supported_cohere_rerank_params(self, model: str) -> list[str]:
        return ["query", "documents"]

    @override
    def map_cohere_rerank_params(
        self,
        non_default_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
        query: str,
        documents: list[str | dict[str, JsonValue]],
        custom_llm_provider: str | None = None,
        top_n: int | None = None,
        rank_fields: list[str] | None = None,
        return_documents: bool | None = True,
        max_chunks_per_doc: int | None = None,
        max_tokens_per_doc: int | None = None,
        instruction: str | None = None,
    ) -> dict[str, JsonValue]:
        return non_default_params


class PreCallRecorder(CustomLogger):
    def __init__(self, events: list[str]) -> None:
        self.events = events
        self.seen: PreCallSeen | None = None

    @override
    def log_pre_api_call(
        self,
        model: str,
        messages: list[dict[str, str]],
        kwargs: dict[str, JsonValue],
    ) -> None:
        self.events.append("pre_call")
        details = PRECALL.validate_python(kwargs)
        additional = details["additional_args"]
        self.seen = PreCallSeen(
            input=details["input"], body=additional["complete_input_dict"]
        )


HANDLER: BaseLLMHTTPHandler = BaseLLMHTTPHandler()


def logging_fixture() -> Logging:
    return Logging(
        model="",
        messages=[],
        stream=False,
        call_type="rerank",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="rerank-http",
        function_id="rerank-http",
    )


async def run_work(work: Coroutine[object, object, RerankResponse]) -> RerankResponse:
    return await work


def settle(work: RerankWork) -> RerankResponse:
    match work:
        case Coroutine():
            return asyncio.run(run_work(work))
        case object():
            return work


def run_once(call: TransportCall, mode: Mode, api_base: str) -> RunOutcome:
    config = RecordingConfig()
    events = config.events
    recorder = PreCallRecorder(events)
    logging_obj = logging_fixture()
    logging_obj.add_dynamic_callback(recorder)
    try:
        work = HANDLER.rerank(
            model=call.model,
            custom_llm_provider="cohere",
            logging_obj=logging_obj,
            provider_config=config,
            optional_rerank_params=call.optional_params,
            timeout=None if call.timeout_seconds is None else float(call.timeout_seconds),
            model_response=RerankResponse(),
            _is_async=mode == "async",
            api_key=call.api_key,
            api_base=api_base,
        )
        record = settle(work)
    except BaseLLMException as error:
        return RunOutcome(
            failure=Failure(
                class_name=type(error).__name__,
                status=error.status_code,
                message=str(error),
            ),
            events=list(events),
            pre_call=recorder.seen,
            transform=config.seen,
        )
    return RunOutcome(
        result=JSON.validate_json(record.model_dump_json()),
        events=list(events),
        pre_call=recorder.seen,
        transform=config.seen,
    )


def run_transport(call: TransportCall, api_base: str) -> Outcome:
    runs = [run_once(call, mode, api_base) for mode in call.modes]
    return Outcome(runs=runs)


class RerankHttpRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    rerank_http: TransportCall

    def serialize(self, api_base: str) -> str:
        outcome = run_transport(self.rerank_http, api_base)
        envelope = TransportEnvelope(rerank_http=TransportValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)
