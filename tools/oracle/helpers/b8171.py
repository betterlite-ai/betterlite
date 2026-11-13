from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.dashscope.common_utils import resolve_dashscope_family_rerank_api_base
from litellm.llms.dashscope.embed.transformation import DashScopeEmbeddingConfig
from litellm.llms.dashscope.rerank.transformation import DashScopeRerankConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import EmbeddingResponse, RerankResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

EMBEDDING_MODEL = "text-embedding-v4"
RERANK_MODEL = "qwen3-rerank"
KEEP_PARAMS: Final = False


class NoMessages(list[AllMessageValues]): ...


class Recorder(Logging):
    def __init__(self) -> None:
        pass

    @override
    def post_call(
        self,
        original_response: JsonValue,
        input: JsonValue = None,
        api_key: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        del original_response, input, api_key, additional_args


class EmbeddingConfig(Protocol):
    def map_openai_params(
        self,
        _non_default_params: dict[str, JsonValue],
        _optional_params: dict[str, JsonValue],
        _model: str,
        _drop_params: Literal[False],
        /,
    ) -> dict[str, JsonValue]: ...

    def validate_environment(
        self,
        _headers: dict[str, str],
        _model: str,
        _messages: NoMessages,
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        _api_key: str | None,
        /,
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
    ) -> str: ...

    def transform_embedding_response(
        self,
        _model: str,
        _raw_response: httpx.Response,
        _model_response: EmbeddingResponse,
        _logging_obj: Recorder,
        _api_key: str | None,
        _request_data: dict[str, JsonValue],
        _optional_params: dict[str, JsonValue],
        _litellm_params: dict[str, JsonValue],
        /,
    ) -> EmbeddingResponse: ...


class RerankConfig(Protocol):
    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(self, api_base: str | None, model: str, optional_params: None) -> str: ...

    def map_cohere_rerank_params(
        self,
        _non_default_params: dict[str, JsonValue] | None,
        _model: str,
        _drop_params: Literal[False],
        _query: str,
        _documents: list[str | dict[str, JsonValue]],
        /,
        *,
        top_n: int | None,
        rank_fields: list[str] | None,
        return_documents: Literal[True, False] | None,
    ) -> dict[str, JsonValue]: ...

    def transform_rerank_request(
        self,
        model: str,
        optional_rerank_params: dict[str, JsonValue],
        headers: dict[str, str],
        litellm_params: None,
    ) -> dict[str, JsonValue]: ...

    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: Recorder,
    ) -> RerankResponse: ...


EMBEDDING: EmbeddingConfig = DashScopeEmbeddingConfig()
RERANK: RerankConfig = DashScopeRerankConfig()


class EmbeddingParamsAndUrlInput(TypedDict):
    function: Literal["embedding_params_and_url"]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    api_base: str | None
    api_key: str | None
    caller_headers: dict[str, str]


class EmbeddingReplyInput(TypedDict):
    function: Literal["embedding_reply"]
    body: str
    status: int
    request_model: str


class RerankBaseInput(TypedDict):
    function: Literal["rerank_base"]
    api_base: str | None
    default_rerank_base: str


class RerankUrlAndHeadersInput(TypedDict):
    function: Literal["rerank_url_and_headers"]
    api_base: str | None
    api_key: str | None
    caller_headers: dict[str, str]


class RerankRequestInput(TypedDict):
    function: Literal["rerank_request"]
    model: str
    query: str
    documents: list[str | dict[str, JsonValue]]
    top_n: int | None
    return_documents: bool | None
    rank_fields: list[str] | None


class RerankReplyInput(TypedDict):
    function: Literal["rerank_reply"]
    body: str
    status: int


class EmbeddingReplyDump(TypedDict):
    object: str
    data: list[JsonValue]
    model: str | None
    usage: dict[str, JsonValue] | None
    id: NotRequired[str]


class RerankReplyDump(TypedDict):
    id: str | None
    results: list[JsonValue]
    meta: dict[str, JsonValue] | None


Input = (
    EmbeddingParamsAndUrlInput
    | EmbeddingReplyInput
    | RerankBaseInput
    | RerankUrlAndHeadersInput
    | RerankRequestInput
    | RerankReplyInput
)


def run(input_value: Input) -> dict[str, JsonValue]:
    if input_value["function"] == "embedding_params_and_url":
        return embedding_params_and_url(input_value)
    if input_value["function"] == "embedding_reply":
        return embedding_reply(input_value)
    if input_value["function"] == "rerank_base":
        return rerank_base(input_value)
    if input_value["function"] == "rerank_url_and_headers":
        return rerank_url_and_headers(input_value)
    if input_value["function"] == "rerank_request":
        return rerank_request(input_value)
    return rerank_reply(input_value)


def embedding_params_and_url(
    input_value: EmbeddingParamsAndUrlInput,
) -> dict[str, JsonValue]:
    mapped = EMBEDDING.map_openai_params(
        input_value["non_default_params"],
        input_value["optional_params"],
        EMBEDDING_MODEL,
        KEEP_PARAMS,
    )
    headers = EMBEDDING.validate_environment(
        input_value["caller_headers"], EMBEDDING_MODEL, NoMessages(), {}, {}, input_value["api_key"]
    )
    url = EMBEDDING.get_complete_url(
        api_base=input_value["api_base"],
        api_key=None,
        model=EMBEDDING_MODEL,
        optional_params={},
        litellm_params={},
    )
    return {"params": mapped, "url": url, "headers": headers}


def embedding_reply(input_value: EmbeddingReplyInput) -> dict[str, JsonValue]:
    raw_response = httpx.Response(input_value["status"], content=input_value["body"].encode())
    result = EMBEDDING.transform_embedding_response(
        input_value["request_model"],
        raw_response,
        EmbeddingResponse(),
        Recorder(),
        None,
        {},
        {},
        {},
    )
    dumped = TypeAdapter(EmbeddingReplyDump).validate_json(result.model_dump_json())
    response: dict[str, JsonValue] = {
        "object": dumped["object"],
        "data": dumped["data"],
        "model": dumped["model"],
        "usage": dumped["usage"],
    }
    return {"response": response, "id": dumped.get("id")}


def rerank_base(input_value: RerankBaseInput) -> dict[str, JsonValue]:
    base = resolve_dashscope_family_rerank_api_base(
        input_value["api_base"], "DASHSCOPE_API_BASE_RERANK", input_value["default_rerank_base"]
    )
    return {"base": base}


def rerank_url_and_headers(
    input_value: RerankUrlAndHeadersInput,
) -> dict[str, JsonValue]:
    return {
        "url": RERANK.get_complete_url(
            api_base=input_value["api_base"], model=RERANK_MODEL, optional_params=None
        ),
        "headers": RERANK.validate_environment(
            headers=input_value["caller_headers"],
            model=RERANK_MODEL,
            api_key=input_value["api_key"],
        ),
    }


def rerank_request(input_value: RerankRequestInput) -> dict[str, JsonValue]:
    params = RERANK.map_cohere_rerank_params(
        {},
        input_value["model"],
        KEEP_PARAMS,
        input_value["query"],
        input_value["documents"],
        top_n=input_value["top_n"],
        rank_fields=input_value["rank_fields"],
        return_documents=input_value["return_documents"],
    )
    body = RERANK.transform_rerank_request(
        model=input_value["model"], optional_rerank_params=params, headers={}, litellm_params=None
    )
    return {"params": params, "body": body}


def rerank_reply(input_value: RerankReplyInput) -> dict[str, JsonValue]:
    raw_response = httpx.Response(input_value["status"], content=input_value["body"].encode())
    result = RERANK.transform_rerank_response(
        model=RERANK_MODEL,
        raw_response=raw_response,
        model_response=RerankResponse(),
        logging_obj=Recorder(),
    )
    dumped = TypeAdapter(RerankReplyDump).validate_json(result.model_dump_json())
    return {"id": dumped["id"], "results": dumped["results"], "meta": dumped["meta"]}
