import logging
from collections.abc import Coroutine
from typing import Final, Literal, TypeAlias

import litellm
from litellm.exceptions import APIConnectionError
from litellm.types.rerank import RerankResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

PREFIX: Final = "Error in rerank: "
MOCK: Final = "{mock}"

Provider: TypeAlias = Literal[
    "cohere",
    "together_ai",
    "azure_ai",
    "infinity",
    "litellm_proxy",
    "hosted_vllm",
    "deepinfra",
    "fireworks_ai",
    "voyage",
    "watsonx",
]
Document: TypeAlias = str | dict[str, JsonValue]
Work: TypeAlias = RerankResponse | Coroutine[object, object, RerankResponse]

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
KWARGS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])


class AsyncCallError(TypeError):
    def __init__(self) -> None:
        super().__init__("litellm.rerank returned a coroutine for the sync dispatcher")


class Settings(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None = None
    api_base: str | None = None
    headers: dict[str, str] = Field(default_factory=dict)
    togetherai_api_key: str | None = None
    drop_params: bool | None = None


SETTINGS: TypeAdapter[Settings] = TypeAdapter(Settings)


class RerankCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    query: str
    documents: list[Document]
    custom_llm_provider: Provider | None = None
    top_n: int | None = None
    rank_fields: list[str] | None = None
    return_documents: bool | None = True
    max_chunks_per_doc: int | None = None
    max_tokens_per_doc: int | None = None
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)
    litellm_settings: Settings = Field(default_factory=Settings)


class Failure(BaseModel):
    model_config = ConfigDict(extra="forbid")

    class_name: str = Field(serialization_alias="class")
    status: int
    kind: str | None = Field(default=None, serialization_alias="type")
    param: str | None = None
    code: str | None = None
    message: str


class Diagnostic(BaseModel):
    model_config = ConfigDict(extra="forbid")

    level: str
    prefix: str


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    result: JsonValue = None
    failure: Failure | None = None
    diagnostics: list[Diagnostic]


class OutcomeValue(BaseModel):
    value: Outcome


class Envelope(BaseModel):
    rerank_provider_plan: OutcomeValue


class Recorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.diagnostics: list[Diagnostic] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(PREFIX):
            self.diagnostics.append(Diagnostic(level=record.levelname.lower(), prefix=PREFIX))


def substituted_kwargs(kwargs: dict[str, JsonValue], api_base: str) -> dict[str, JsonValue]:
    document = KWARGS.dump_json(kwargs).decode().replace(MOCK, api_base)
    return KWARGS.validate_json(document)


def apply_settings(settings: Settings) -> None:
    litellm.suppress_debug_info = True
    litellm.api_key = settings.api_key
    litellm.api_base = settings.api_base
    litellm.togetherai_api_key = settings.togetherai_api_key
    if settings.drop_params is not None:
        litellm.drop_params = settings.drop_params


def connection_failure(error: APIConnectionError) -> Failure:
    return Failure(
        class_name=type(error).__name__,
        status=error.status_code,
        kind=error.type,
        param=error.param,
        code=error.code,
        message=error.message,
    )


def call_outcome(call: RerankCall, api_base: str, recorder: Recorder) -> Outcome:
    kwargs = substituted_kwargs(call.kwargs, api_base)
    try:
        work: Work = litellm.rerank(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/main.py#L91-L92
            model=call.model,
            query=call.query,
            documents=call.documents,
            custom_llm_provider=call.custom_llm_provider,
            top_n=call.top_n,
            rank_fields=call.rank_fields,
            return_documents=call.return_documents,
            max_chunks_per_doc=call.max_chunks_per_doc,
            max_tokens_per_doc=call.max_tokens_per_doc,
            **kwargs,
        )
    except APIConnectionError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L853-L861
        return Outcome(
            failure=connection_failure(error),
            diagnostics=list(recorder.diagnostics),
        )
    match work:
        case Coroutine():
            raise AsyncCallError
        case object():
            return Outcome(
                result=JSON.validate_json(work.model_dump_json()),
                diagnostics=list(recorder.diagnostics),
            )


class RerankProviderPlanRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    rerank_provider_plan: RerankCall

    def serialize(self, api_base: str) -> str:
        call = self.rerank_provider_plan
        settings = SETTINGS.validate_json(
            call.litellm_settings.model_dump_json().replace(MOCK, api_base)
        )
        apply_settings(settings)
        recorder = Recorder()
        logger = logging.getLogger("LiteLLM")
        logger.addHandler(recorder)
        try:
            outcome = call_outcome(call, api_base, recorder)
        finally:
            logger.removeHandler(recorder)
        envelope = Envelope(rerank_provider_plan=OutcomeValue(value=outcome))
        return envelope.model_dump_json(by_alias=True)
