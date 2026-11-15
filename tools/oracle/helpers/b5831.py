import contextlib
import io
from collections.abc import Mapping
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.together_ai import cost_calculator
from litellm.llms.together_ai.chat.transformation import TogetherAIChatConfig
from litellm.llms.together_ai.completion.transformation import TogetherAITextCompletionConfig
from litellm.llms.together_ai.rerank.handler import TogetherAIRerank
from litellm.types.llms.openai import OpenAITextCompletionUserMessage as UserMessage
from litellm.types.rerank import RerankResponse
from litellm.types.utils import CallTypes
from openai import APIStatusError
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

INTEGERS: Final = "TogetherAI does not support integers as input"
MODEL: Final = "meta-llama/Llama-3.3-70B-Instruct-Turbo"
OBJECT = TypeAdapter(dict[str, JsonValue])
USAGE: dict[str, JsonValue] = {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
CHOICE: dict[str, JsonValue] = {"index": 0, "finish_reason": "stop"}
CHAT_REPLY: dict[str, JsonValue] = {
    "choices": [{**CHOICE, "message": {"role": "assistant", "content": "hi"}}],
    "usage": USAGE,
}
CALL_TYPES: dict[str, CallTypes] = {
    "embedding": CallTypes.embedding,
    "aembedding": CallTypes.aembedding,
    "other": CallTypes.completion,
}


class CapturedClient:
    def __init__(self, reply: dict[str, JsonValue]) -> None:
        self.reply = reply
        self.url = ""
        self.headers: dict[str, JsonValue] = {}
        self.payload: JsonValue = None

    def post(self, url: str, headers: dict[str, str], json: JsonValue) -> httpx.Response:
        self.url = url
        self.headers = OBJECT.validate_python(headers)
        self.payload = json
        return httpx.Response(200, content=OBJECT.dump_json(self.reply))


class Recorder:
    def __init__(self, reply: dict[str, JsonValue]) -> None:
        self.reply = reply
        self.body: dict[str, JsonValue] = {}

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.body = OBJECT.validate_json(request.content)
        return httpx.Response(200, json=self.reply)


class ClientCache(LLMClientCache):
    def __init__(self, client: CapturedClient) -> None:
        super().__init__()
        self.client = client

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> CapturedClient:
        return self.client


class ModelStep(TypedDict):
    model: str


class ChatMapStep(TypedDict):
    model: str
    params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    drop_params: NotRequired[bool]
    global_drop_params: NotRequired[bool]


class ChatMessagesStep(TypedDict):
    messages: list[dict[str, JsonValue]]


class TextMessage(TypedDict):
    role: Literal["user"]
    content: str | list[str] | list[int] | list[list[int]] | None


class CompletionStep(TypedDict):
    model: str
    messages: list[TextMessage]
    optional_params: dict[str, JsonValue]


class RerankRequestStep(TypedDict):
    model: str
    query: str
    documents: list[str | dict[str, JsonValue]]
    top_n: NotRequired[int]
    rank_fields: NotRequired[list[str]]
    return_documents: NotRequired[bool | None]
    max_chunks_per_doc: NotRequired[int]
    api_key: str
    api_base: str


class RerankResponseStep(TypedDict):
    response: dict[str, JsonValue]
    next_id: str


class CategoryStep(TypedDict):
    model: str
    call_type: Literal["embedding", "aembedding", "other"]


class Input(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    chat_supported: list[ModelStep]
    chat_map: list[ChatMapStep]
    chat_messages: list[ChatMessagesStep]
    completion: list[CompletionStep]
    rerank_requests: list[RerankRequestStep]
    rerank_responses: list[RerankResponseStep]
    cost_categories: list[CategoryStep]
    cost_registry: list[ModelStep]


INPUT = TypeAdapter(Input)


class CostCalculator(Protocol):
    def get_model_params_and_category(self, model_name: str, call_type: CallTypes) -> str: ...

    def has_together_registry_pricing(
        self, model: str, cost_map: Mapping[str, JsonValue]
    ) -> bool: ...


COSTS: CostCalculator = cost_calculator


class TextConfig(Protocol):
    def transform_text_completion_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[UserMessage],
        optional_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


CHAT: ChatConfig = TogetherAIChatConfig()
TEXT: TextConfig = TogetherAITextCompletionConfig()


def chat_supported(step: ModelStep) -> JsonValue:
    params: list[JsonValue] = [*CHAT.get_supported_openai_params(step["model"])]
    return params


def chat_map(step: ChatMapStep) -> JsonValue:
    litellm.drop_params = step.get("global_drop_params") is True
    try:
        mapped = CHAT.map_openai_params(
            step["params"],
            step["optional_params"],
            step["model"],
            drop_params=step.get("drop_params") is True,
        )
    except APIStatusError as error:
        return {"ok": False, "error": {"status": error.status_code}}
    return {"ok": True, "value": mapped}


def chat_messages(step: ChatMessagesStep) -> JsonValue:
    recorder = Recorder(CHAT_REPLY)
    litellm.completion(
        model=f"together_ai/{MODEL}",
        messages=step["messages"],
        client=HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.handle))),
        api_key="k",
    )
    return recorder.body["messages"]


def completion(step: CompletionStep) -> JsonValue:
    messages = [
        UserMessage(role="user", content=message["content"]) for message in step["messages"]
    ]
    try:
        request = TEXT.transform_text_completion_request(
            step["model"], messages, step["optional_params"], {}
        )
    except ValueError as error:
        kind = "integers" if INTEGERS in str(error) else "multiple_prompts"
        return {"ok": False, "error": {"kind": kind}}
    return {"ok": True, "value": request}


def rerank(client: CapturedClient, step: RerankRequestStep) -> RerankResponse:
    litellm.in_memory_llm_clients_cache = ClientCache(client)
    returned = step.get("return_documents")
    if "return_documents" not in step:
        returned = True
    return TogetherAIRerank().rerank(
        step["model"],
        step["api_key"],
        step["api_base"],
        step["query"],
        step["documents"],
        step.get("top_n"),
        step.get("rank_fields"),
        returned,
        step.get("max_chunks_per_doc"),
    )


def rerank_request(step: RerankRequestStep) -> JsonValue:
    client = CapturedClient({"results": []})
    try:
        rerank(client, step)
    except ValueError:
        return {"ok": False, "error": {"kind": "max_chunks"}}
    sent: JsonValue = {"url": client.url, "headers": client.headers, "body": client.payload}
    return {"ok": True, "value": sent}


def rerank_response(step: RerankResponseStep) -> JsonValue:
    response = step["response"]
    probe: RerankRequestStep = {
        "model": "m",
        "query": "q",
        "documents": [],
        "api_key": "k",
        "api_base": "https://together.test",
    }
    try:
        reply = rerank(CapturedClient(response), probe)
    except ValueError as error:
        kind = "no_results" if str(error).startswith("No results found") else "missing_fields"
        return {"ok": False, "error": {"kind": kind}}
    dumped = OBJECT.validate_json(reply.model_dump_json(exclude_none=True))
    if not response.get("id"):
        dumped["id"] = step["next_id"]
    return {"ok": True, "value": dumped}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    registered = {key: dict(entry) for key, entry in parsed["model_cost"].items()}
    with contextlib.redirect_stdout(io.StringIO()):
        litellm.register_model(registered, persist_across_reloads=False)
    collected: dict[str, JsonValue] = {}
    collected["chat_supported"] = [chat_supported(step) for step in parsed["chat_supported"]]
    collected["chat_map"] = [chat_map(step) for step in parsed["chat_map"]]
    collected["chat_messages"] = [chat_messages(step) for step in parsed["chat_messages"]]
    collected["completion"] = [completion(step) for step in parsed["completion"]]
    collected["rerank_requests"] = [rerank_request(step) for step in parsed["rerank_requests"]]
    collected["rerank_responses"] = [rerank_response(step) for step in parsed["rerank_responses"]]
    collected["cost_categories"] = [
        COSTS.get_model_params_and_category(step["model"], CALL_TYPES[step["call_type"]])
        for step in parsed["cost_categories"]
    ]
    collected["cost_registry"] = [
        COSTS.has_together_registry_pricing(step["model"], parsed["model_cost"])
        for step in parsed["cost_registry"]
    ]
    return collected
