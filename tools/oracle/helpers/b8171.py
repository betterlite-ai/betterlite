from typing import Literal, NotRequired, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.dashscope.common_utils import resolve_dashscope_family_rerank_api_base
from litellm.llms.dashscope.embed.transformation import DashScopeEmbeddingConfig
from litellm.llms.dashscope.rerank.transformation import DashScopeRerankConfig
from litellm.types.utils import EmbeddingResponse, RerankResponse
from pydantic import JsonValue, TypeAdapter

EMBEDDING = DashScopeEmbeddingConfig()
RERANK = DashScopeRerankConfig()
EMBEDDING_MODEL = "text-embedding-v4"
RERANK_MODEL = "qwen3-rerank"


class EmbeddingParamsAndUrlInput(TypedDict):
    function: Literal["embedding_params_and_url"]
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    api_base: str | None
    env_base: str | None
    api_key: str | None
    env_key: str | None
    caller_headers: dict[str, str]


class EmbeddingReplyInput(TypedDict):
    function: Literal["embedding_reply"]
    body: str
    status: int
    request_model: str


class RerankBaseInput(TypedDict):
    function: Literal["rerank_base"]
    api_base: str | None
    env_value: str
    default_rerank_base: str


class RerankUrlAndHeadersInput(TypedDict):
    function: Literal["rerank_url_and_headers"]
    api_base: str | None
    env_value: str
    default_rerank_url: str
    api_key: str | None
    env_key: str | None
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
        dict(input_value["non_default_params"]),
        dict(input_value["optional_params"]),
        EMBEDDING_MODEL,
        drop_params=False,
    )
    headers = EMBEDDING.validate_environment(
        dict(input_value["caller_headers"]),
        EMBEDDING_MODEL,
        [],
        {},
        {},
        api_key=input_value["api_key"],
    )
    url = EMBEDDING.get_complete_url(
        input_value["api_base"], None, EMBEDDING_MODEL, {}, {}
    )
    params = TypeAdapter(dict[str, JsonValue]).validate_python(mapped)
    resolved = TypeAdapter(dict[str, str]).validate_python(headers)
    return {"params": params, "url": url, "headers": resolved}


def embedding_reply(input_value: EmbeddingReplyInput) -> dict[str, JsonValue]:
    raw_response = httpx.Response(
        input_value["status"], content=input_value["body"].encode()
    )
    logging_obj = object.__new__(Logging)
    logging_obj.model_call_details = {}
    result = EMBEDDING.transform_embedding_response(
        input_value["request_model"],
        raw_response,
        EmbeddingResponse(),
        logging_obj,
        None,
        {},
        {},
        {},
    )
    dumped = TypeAdapter(EmbeddingReplyDump).validate_json(result.model_dump_json())
    usage = dumped["usage"]
    usage_fields: dict[str, JsonValue] | None = None
    if usage is not None:
        usage_fields = {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
            "completion_tokens_details": None,
            "prompt_tokens_details": None,
        }
    response: dict[str, JsonValue] = {
        "object": dumped["object"],
        "data": dumped["data"],
        "model": dumped["model"],
        "usage": usage_fields,
    }
    return {"response": response, "id": dumped.get("id")}


def rerank_base(input_value: RerankBaseInput) -> dict[str, JsonValue]:
    base = resolve_dashscope_family_rerank_api_base(
        input_value["api_base"],
        "DASHSCOPE_API_BASE_RERANK",
        input_value["default_rerank_base"],
    )
    return {"base": base}


def rerank_url_and_headers(
    input_value: RerankUrlAndHeadersInput,
) -> dict[str, JsonValue]:
    headers = RERANK.validate_environment(
        dict(input_value["caller_headers"]),
        RERANK_MODEL,
        api_key=input_value["api_key"],
    )
    url = RERANK.get_complete_url(input_value["api_base"], RERANK_MODEL, None)
    resolved = TypeAdapter(dict[str, str]).validate_python(headers)
    return {"url": url, "headers": resolved}


def rerank_request(input_value: RerankRequestInput) -> dict[str, JsonValue]:
    params = RERANK.map_cohere_rerank_params(
        {},
        input_value["model"],
        False,
        input_value["query"],
        input_value["documents"],
        top_n=input_value["top_n"],
        rank_fields=input_value["rank_fields"],
        return_documents=input_value["return_documents"],
    )
    body = RERANK.transform_rerank_request(
        input_value["model"], dict(params), {}, None
    )
    mapped = TypeAdapter(dict[str, JsonValue]).validate_python(params)
    request_body = TypeAdapter(dict[str, JsonValue]).validate_python(body)
    return {"params": mapped, "body": request_body}


def rerank_reply(input_value: RerankReplyInput) -> dict[str, JsonValue]:
    raw_response = httpx.Response(
        input_value["status"], content=input_value["body"].encode()
    )
    logging_obj = object.__new__(Logging)
    logging_obj.model_call_details = {}
    result = RERANK.transform_rerank_response(
        RERANK_MODEL,
        raw_response,
        RerankResponse(),
        logging_obj,
        None,
        {},
        {},
        {},
    )
    dumped = TypeAdapter(RerankReplyDump).validate_json(result.model_dump_json())
    return {"id": dumped["id"], "results": dumped["results"], "meta": dumped["meta"]}
