import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime
from decimal import Decimal
from typing import Final, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure_ai.rerank.transformation import AzureAIRerankConfig
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter, ValidationError

JSON = TypeAdapter(JsonValue)
HIDDEN = TypeAdapter(dict[str, JsonValue])
Params = dict[str, JsonValue]
Document = str | dict[str, JsonValue]
MODEL: Final = "cohere-rerank-v3-multilingual"
AUTH_VARIABLES: Final = (
    "AZURE_AI_API_KEY",
    "AZURE_AD_TOKEN",
    "AZURE_TENANT_ID",
    "AZURE_CLIENT_ID",
    "AZURE_CLIENT_SECRET",
    "AZURE_USERNAME",
    "AZURE_PASSWORD",
    "AZURE_SCOPE",
)

MapParams = Callable[
    [
        Params,
        str,
        bool,
        str,
        list[Document],
        str | None,
        int | None,
        list[str] | None,
        bool | None,
        int | None,
    ],
    Params,
]
class Config(Protocol):
    @property
    def map_cohere_rerank_params(self) -> MapParams: ...

    def get_complete_url(
        self, api_base: str | None, model: str, optional_params: Params
    ) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None,
        optional_params: Params | None = None,
        litellm_params: Params | None = None,
    ) -> dict[str, str]: ...

    def transform_rerank_request(
        self, model: str, optional_rerank_params: Params, headers: dict[str, str]
    ) -> Params: ...

    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: Logging,
    ) -> RerankResponse: ...

    def _get_base_model(self, azure_model_group: str | None) -> str | None: ...


class ResultView(TypedDict):
    index: int
    relevance_score: float
    document: NotRequired[dict[str, JsonValue]]


class ReplyView(TypedDict):
    id: str | None
    results: list[ResultView] | None
    meta: dict[str, JsonValue] | None


class UrlCall(TypedDict, total=False):
    api_base: str | None


class RequestCall(TypedDict):
    model: str
    query: str
    documents: NotRequired[list[Document]]
    top_n: NotRequired[int | None]
    rank_fields: NotRequired[list[str] | None]
    return_documents: NotRequired[bool | None]
    max_chunks_per_doc: NotRequired[int | None]


class ReplyCall(TypedDict):
    status: int
    body: str
    model_group: NotRequired[str | None]


class AuthCall(TypedDict, total=False):
    api_key: str | None
    global_api_key: str | None
    params: Params | None
    headers: dict[str, str]
    environment: dict[str, str]


class Input(TypedDict, total=False):
    urls: list[UrlCall]
    requests: list[RequestCall]
    replies: list[ReplyCall]
    auths: list[AuthCall]


REPLY = TypeAdapter(ReplyView)
CONFIG: Config = AzureAIRerankConfig()


@contextmanager
def auth_environment(values: dict[str, str]) -> Generator[None]:
    saved = {name: os.environ.get(name) for name in AUTH_VARIABLES}
    for name in AUTH_VARIABLES:
        os.environ.pop(name, None)
    for name, value in values.items():
        os.environ[name] = value
    try:
        yield
    finally:
        for name in AUTH_VARIABLES:
            os.environ.pop(name, None)
        for name, previous in saved.items():
            if previous is not None:
                os.environ[name] = previous


@contextmanager
def global_key(value: str | None) -> Generator[None]:
    saved_key = litellm.azure_key
    saved_refresh = litellm.enable_azure_ad_token_refresh
    litellm.azure_key = value
    litellm.enable_azure_ad_token_refresh = False
    try:
        yield
    finally:
        litellm.azure_key = saved_key
        litellm.enable_azure_ad_token_refresh = saved_refresh


def url_outcome(call: UrlCall) -> JsonValue:
    try:
        return {"url": CONFIG.get_complete_url(call.get("api_base"), MODEL, {})}
    except (ValueError, httpx.InvalidURL):
        return {"error": {"status": None}}


def request_outcome(call: RequestCall) -> JsonValue:
    documents = call.get("documents")
    if documents is None:
        documents = []
    returned = call.get("return_documents")
    drop_params = False
    mapped = CONFIG.map_cohere_rerank_params(
        {},
        call["model"],
        drop_params,
        call["query"],
        documents,
        None,
        call.get("top_n"),
        call.get("rank_fields"),
        returned is None or returned,
        call.get("max_chunks_per_doc"),
    )
    body = CONFIG.transform_rerank_request(call["model"], mapped, {})
    return {"body": JSON.validate_python(body)}


def reply_outcome(call: ReplyCall) -> JsonValue:
    try:
        answer = CONFIG.transform_rerank_response(
            model=MODEL,
            raw_response=httpx.Response(call["status"], content=call["body"].encode()),
            model_response=RerankResponse(),
            logging_obj=Logging(
                model=MODEL,
                messages=None,
                stream=False,
                call_type="rerank",
                start_time=datetime(2026, 1, 1, tzinfo=UTC),
                litellm_call_id="rerank-call",
                function_id="rerank-function",
            ),
        )
    except BaseLLMException as error:
        return {"error": {"status": error.status_code}}
    except (TypeError, ValidationError):
        return {"error": {"status": None}}
    hidden = HIDDEN.validate_python(get_hidden_params_dict(answer))
    view = REPLY.validate_json(answer.model_dump_json())
    results: list[JsonValue] | None = None
    if view["results"] is not None:
        results = [
            {
                "index": entry["index"],
                "relevance_score": str(Decimal(repr(entry["relevance_score"])).normalize()),
                "document": entry.get("document"),
            }
            for entry in view["results"]
        ]
    return {
        "id": view["id"],
        "results": results,
        "meta": view["meta"],
        "hidden_model": CONFIG._get_base_model(call.get("model_group")),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/rerank/transformation.py#L112-L119
        "hidden_model_present": "model" in hidden,
    }


def auth_outcome(call: AuthCall) -> JsonValue:
    values = call.get("environment")
    if values is None:
        values = {}
    headers = call.get("headers")
    if headers is None:
        headers = {}
    with auth_environment(values), global_key(call.get("global_api_key")):
        try:
            answer = CONFIG.validate_environment(
                headers, MODEL, call.get("api_key"), None, call.get("params")
            )
        except (TypeError, ValueError):
            return {"error": {"status": None}}
    return {"headers": JSON.validate_python(answer)}


def run(input_value: Input) -> JsonValue:
    urls = input_value.get("urls")
    requests = input_value.get("requests")
    replies = input_value.get("replies")
    auths = input_value.get("auths")
    return {
        "urls": [url_outcome(call) for call in urls] if urls is not None else [],
        "requests": [request_outcome(call) for call in requests] if requests is not None else [],
        "replies": [reply_outcome(call) for call in replies] if replies is not None else [],
        "auths": [auth_outcome(call) for call in auths] if auths is not None else [],
    }
