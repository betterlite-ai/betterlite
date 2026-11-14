from collections.abc import Mapping
from datetime import UTC, datetime
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.s3_vectors.vector_stores.transformation import S3VectorsVectorStoreConfig
from litellm.types.vector_stores import VectorStoreResultContent
from pydantic import JsonValue, TypeAdapter

_JSON: Final = TypeAdapter(JsonValue)
_OBJECT: Final = TypeAdapter(dict[str, JsonValue])


class Options(TypedDict, total=False):
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None


class Result(TypedDict, total=False):
    score: float | None
    content: list[VectorStoreResultContent] | None
    file_id: str | None
    filename: str | None
    attributes: dict[str, JsonValue] | None


class Reply(TypedDict, total=False):
    object: Literal["vector_store.search_results.page"]
    search_query: str | None
    data: list[Result] | None


class SearchConfig(Protocol):
    def get_complete_url(
        self, api_base: str | None, litellm_params: dict[str, JsonValue]
    ) -> str: ...

    def query_embedding_model(self, litellm_params: Mapping[str, JsonValue]) -> str: ...

    def query_text(self, query: str) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_request(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/s3_vectors/vector_stores/transformation.py#L143-L156
        self,
        vector_store_id: str,
        query: str,
        # ast-grep-ignore: no-long-names
        vector_store_search_optional_params: Options,
        api_base: str,
        litellm_logging_obj: Logging,
        litellm_params: Mapping[str, JsonValue],
    ) -> tuple[str, dict[str, object]]: ...  # ast-grep-ignore: no-loose-types

    # ast-grep-ignore: no-long-names
    def transform_search_vector_store_response(
        self, response: httpx.Response, litellm_logging_obj: Logging
    ) -> Reply: ...


CONFIG: SearchConfig = S3VectorsVectorStoreConfig()
PROVIDER_ERROR: Final[type[Exception]] = BaseLLMException


class RequestInput(TypedDict):
    vector_store_id: str
    query: str
    vector: list[float]
    max_num_results: NotRequired[int]
    litellm_params: dict[str, JsonValue]


class ResponseInput(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    query: NotRequired[str]


class RequestCall(TypedDict):
    call: Literal["request"]
    input: RequestInput


class ResponseCall(TypedDict):
    call: Literal["response"]
    input: ResponseInput


class Input(TypedDict):
    calls: list[RequestCall | ResponseCall]


def logging_object() -> Logging:
    return Logging(
        model="s3-vectors",
        messages=None,
        stream=False,
        call_type="embedding",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b7812",
        function_id="b7812",
    )


def request(call: RequestInput) -> JsonValue:
    options: Options = {}
    if "max_num_results" in call:
        options["max_num_results"] = call["max_num_results"]
    api_base = CONFIG.get_complete_url(None, call["litellm_params"])
    url, body = CONFIG.transform_search_vector_store_request(
        vector_store_id=call["vector_store_id"],
        query=call["query"],
        vector_store_search_optional_params=options,
        api_base=api_base,
        litellm_logging_obj=logging_object(),
        litellm_params={
            **call["litellm_params"],
            "litellm_embedding_config": {"mock_response": _JSON.validate_python(call["vector"])},
        },
    )
    return {
        "api_base": api_base,
        "url": url,
        "body": _OBJECT.validate_python(body),
        "embedding_model": CONFIG.query_embedding_model(call["litellm_params"]),
        "query": CONFIG.query_text(call["query"]),
    }


def response(call: ResponseInput) -> JsonValue:
    logging = logging_object()
    if "query" in call:
        CONFIG.transform_search_vector_store_request(
            vector_store_id="bucket:index",
            query=call["query"],
            vector_store_search_optional_params=Options(),
            api_base="https://s3vectors.us-west-2.api.aws",
            litellm_logging_obj=logging,
            litellm_params={"litellm_embedding_config": {"mock_response": [0.0]}},
        )
    reply = httpx.Response(call["status"], headers=call["headers"], content=call["body"].encode())
    return _JSON.validate_python(CONFIG.transform_search_vector_store_response(reply, logging))


def run(input_value: Input) -> JsonValue:
    outcomes: list[JsonValue] = []
    for call in input_value["calls"]:
        try:
            if call["call"] == "request":
                outcomes.append({"ok": True, "value": request(call["input"])})
            else:
                outcomes.append({"ok": True, "value": response(call["input"])})
        except (ValueError, PROVIDER_ERROR):
            outcomes.append({"ok": False})
    return outcomes
