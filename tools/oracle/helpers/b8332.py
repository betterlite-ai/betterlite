from collections.abc import Callable, Mapping
from typing import Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.exceptions import APIConnectionError, APIError, BadRequestError, RateLimitError
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.huggingface.rerank.transformation import HuggingFaceRerankConfig
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

KEPT_HEADERS = ("authorization", "content-type")
HUB_HOST = "huggingface.co"
MODEL_INFO_PREFIX = "/api/models/"
# https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/http_handler.py#L1782
REPLICATE_CLIENT_KEY = "httpx_clienttimeout_600.0"
REPLY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

CompleteUrl = Callable[[str | None, str, dict[str, JsonValue] | None], str]
MapParams = Callable[
    [dict[str, JsonValue] | None, str, bool, str, list[str | dict[str, JsonValue]]],
    dict[str, JsonValue],
]
ValidateEnvironment = Callable[
    [
        dict[str, str],
        str,
        str | None,
        dict[str, JsonValue] | None,
        Mapping[str, JsonValue] | None,
        str | None,
    ],
    dict[str, str],
]
TransformRequest = Callable[
    [str, dict[str, JsonValue], dict[str, str], dict[str, JsonValue] | None],
    dict[str, JsonValue],
]


class RerankConfig(Protocol):
    @property
    def get_complete_url(self) -> CompleteUrl: ...

    @property
    def map_cohere_rerank_params(self) -> MapParams: ...

    @property
    def validate_environment(self) -> ValidateEnvironment: ...

    @property
    def transform_rerank_request(self) -> TransformRequest: ...


class Reply(TypedDict):
    status: int
    body: str


class Message(TypedDict):
    role: Literal["user"]
    content: str


class Case(TypedDict):
    op: Literal[
        "huggingface_chat", "huggingface_embedding", "huggingface_rerank", "replicate", "predibase"
    ]
    model: str
    messages: list[Message]
    stream: NotRequired[bool]
    input_type: NotRequired[str]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    input: list[str]
    query: str
    env: dict[str, str]
    replies: list[Reply]


class Recorder:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5849
        if request.url.host == HUB_HOST and not request.url.path.startswith(MODEL_INFO_PREFIX):
            return httpx.Response(404)
        body: JsonValue = None
        if request.content:
            body = REPLY.validate_json(request.content)
        headers: dict[str, JsonValue] = {
            name: value
            for name, value in request.headers.items()
            if name in KEPT_HEADERS or name.startswith("x-")
        }
        url = str(request.url)
        self.requests.append({"method": request.method, "url": url, "headers": headers, "body": body})
        reply = self.replies.pop(0)
        return httpx.Response(reply["status"], content=reply["body"].encode())


def rerank_request(case: Case) -> JsonValue:
    view: RerankConfig = HuggingFaceRerankConfig()
    query = case["query"]
    documents: list[str | dict[str, JsonValue]] = [*case["input"]]
    asked: dict[str, JsonValue] = {"query": query, "documents": [*case["input"]]}
    asked["return_documents"] = True
    mapped = view.map_cohere_rerank_params(asked, case["model"], False, query, documents)
    headers = view.validate_environment({}, case["model"], None, None, None, None)
    kept: dict[str, JsonValue] = {
        name.lower(): value
        for name, value in headers.items()
        if name.lower() in KEPT_HEADERS or name.lower().startswith("x-")
    }
    body = view.transform_rerank_request(case["model"], mapped, headers, None)
    url = view.get_complete_url(None, case["model"], None)
    return {"method": "POST", "url": url, "headers": kept, "body": body}


def provider_call(case: Case, client: HTTPHandler) -> None:
    messages = case["messages"]
    api_base = case.get("api_base")
    api_key = case.get("api_key")
    model = case["model"]
    match case["op"]:
        case "huggingface_chat":
            litellm.completion(
                model=f"huggingface/{model}",
                messages=messages,
                api_base=api_base,
                api_key=api_key,
                stream=case.get("stream"),
                client=client,
            )
        case "huggingface_embedding":
            litellm.embedding(
                model=f"huggingface/{model}",
                input=case["input"],
                input_type=case.get("input_type"),
                api_base=api_base,
                api_key=api_key,
                client=client,
            )
        case "replicate":
            litellm.completion(
                model=f"replicate/{model}", messages=messages, api_base=api_base, api_key=api_key
            )
        case "predibase":
            litellm.completion(
                model=f"predibase/{model}", messages=messages, api_base=api_base, api_key=api_key
            )
        case "huggingface_rerank":
            pass


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    if case["op"] == "huggingface_rerank":
        return {"requests": [rerank_request(case)], "error": None}
    recorder = Recorder(case["replies"])
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle)))
    litellm.module_level_client = client
    litellm.in_memory_llm_clients_cache = LLMClientCache()
    litellm.in_memory_llm_clients_cache.set_cache(REPLICATE_CLIENT_KEY, client)
    error: JsonValue = None
    try:
        provider_call(case, client)
    except (APIError, APIConnectionError, BadRequestError, RateLimitError) as raised:
        error = {"status": raised.status_code}
    return {"requests": recorder.requests, "error": error}
