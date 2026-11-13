import contextlib
import io
from typing import Literal, NotRequired, Protocol, TypedDict

import litellm
from litellm.llms.together_ai.chat.transformation import (
    LITELLM_INTERNAL_ASSISTANT_FIELDS,
    TogetherAIChatConfig,
)
from litellm.llms.together_ai.completion.transformation import (
    TogetherAITextCompletionConfig,
)
from openai import APIError
from litellm.llms.together_ai.cost_calculator import (
    get_model_params_and_category,
    has_together_registry_pricing,
)
from litellm.types.llms.openai import OpenAITextCompletionUserMessage
from litellm.types.rerank import RerankRequest
from litellm.types.utils import CallTypes
from pydantic import JsonValue, TypeAdapter

INTEGERS = "TogetherAI does not support integers as input"
MULTIPLE = "TogetherAI does not support multiple prompts."

INPUT_MESSAGES = TypeAdapter(list[OpenAITextCompletionUserMessage])
OBJECT = TypeAdapter(dict[str, JsonValue])
USAGE = TypeAdapter(dict[str, int | None])
RESULTS = TypeAdapter(list[dict[str, JsonValue]])
IDENTIFIER = TypeAdapter(str | None)

CALL_TYPES: dict[str, CallTypes] = {
    "embedding": CallTypes.embedding,
    "aembedding": CallTypes.aembedding,
    "other": CallTypes.completion,
}


class ModelCostEntry(TypedDict, total=False):
    litellm_provider: str
    mode: str
    supports_function_calling: bool
    supports_response_schema: bool
    supports_reasoning: bool
    reasoning_effort_levels: list[str]
    input_cost_per_token: float


class ChatSupportedStep(TypedDict):
    model: str


class ChatMapStep(TypedDict):
    model: str
    params: dict[str, JsonValue]
    optional_params: NotRequired[dict[str, JsonValue]]
    drop_params: NotRequired[bool]
    global_drop_params: NotRequired[bool]


class ChatMessagesStep(TypedDict):
    messages: list[dict[str, JsonValue]]


class CompletionStep(TypedDict):
    model: str
    messages: list[JsonValue]
    optional_params: NotRequired[dict[str, JsonValue]]


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


class ResultFields(TypedDict, total=False):
    index: int
    relevance_score: float
    document: dict[str, JsonValue]


RESULT = TypeAdapter(ResultFields)


class CategoryStep(TypedDict):
    model: str
    call_type: NotRequired[Literal["embedding", "aembedding", "other"]]


class RegistryStep(TypedDict):
    model: str


class Input(TypedDict):
    model_cost: NotRequired[dict[str, ModelCostEntry]]
    chat_supported: NotRequired[list[ChatSupportedStep]]
    chat_map: NotRequired[list[ChatMapStep]]
    chat_messages: NotRequired[list[ChatMessagesStep]]
    completion: NotRequired[list[CompletionStep]]
    rerank_requests: NotRequired[list[RerankRequestStep]]
    rerank_responses: NotRequired[list[RerankResponseStep]]
    cost_categories: NotRequired[list[CategoryStep]]
    cost_registry: NotRequired[list[RegistryStep]]


INPUT = TypeAdapter(Input)


class ChatConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


CHAT: ChatConfig = TogetherAIChatConfig()


class TextCompletionConfig(Protocol):
    def transform_text_completion_request(
        self,
        model: str,
        messages: list[OpenAITextCompletionUserMessage],
        optional_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...


TEXT: TextCompletionConfig = TogetherAITextCompletionConfig()


def chat_supported(step: ChatSupportedStep) -> JsonValue:
    params: list[JsonValue] = [*CHAT.get_supported_openai_params(step["model"])]
    return params


def chat_map(step: ChatMapStep) -> JsonValue:
    litellm.drop_params = step.get("global_drop_params") is True
    optional = step.get("optional_params", {})
    try:
        mapped = CHAT.map_openai_params(
            step["params"], optional, step["model"], step.get("drop_params") is True
        )
    except APIError as error:
        return {"ok": False, "error": {"status": error.status_code}}
    return {"ok": True, "value": mapped}


def chat_messages(step: ChatMessagesStep) -> JsonValue:
    stripped: list[JsonValue] = []
    for message in step["messages"]:
        if message.get("role") == "assistant":
            stripped.append(
                {
                    key: value
                    for key, value in message.items()
                    if key not in LITELLM_INTERNAL_ASSISTANT_FIELDS
                }
            )
        else:
            stripped.append(message)
    return stripped


def prompt_kind(message: str) -> str:
    if message == INTEGERS:
        return "integers"
    return "multiple_prompts"


def completion(step: CompletionStep) -> JsonValue:
    messages = INPUT_MESSAGES.validate_python(step["messages"])
    try:
        request = TEXT.transform_text_completion_request(
            step["model"], messages, step.get("optional_params", {}), {}
        )
    except ValueError as error:
        return {"ok": False, "error": {"kind": prompt_kind(str(error))}}
    return {"ok": True, "value": request}


def rerank_request(step: RerankRequestStep) -> JsonValue:
    request = RerankRequest(
        model=step["model"],
        query=step["query"],
        top_n=step.get("top_n"),
        documents=step["documents"],
        rank_fields=step.get("rank_fields"),
        return_documents=step.get("return_documents", True),
    )
    body = OBJECT.validate_json(request.model_dump_json(exclude_none=True))
    if step.get("max_chunks_per_doc") is not None:
        return {"ok": False, "error": {"kind": "max_chunks"}}
    return {
        "ok": True,
        "value": {
            "url": step["api_base"].rstrip("/") + "/rerank",
            "headers": {
                "accept": "application/json",
                "content-type": "application/json",
                "authorization": "Bearer " + step["api_key"],
            },
            "body": body,
        },
    }


def usage_blocks(usage: dict[str, int | None]) -> dict[str, JsonValue]:
    billed: dict[str, JsonValue] = {}
    if "search_units" in usage:
        billed["search_units"] = usage["search_units"]
    if "total_tokens" in usage:
        billed["total_tokens"] = usage["total_tokens"]
    tokens: dict[str, JsonValue] = {}
    if "input_tokens" in usage:
        tokens["input_tokens"] = usage["input_tokens"]
    if "output_tokens" in usage:
        tokens["output_tokens"] = usage["output_tokens"]
    return {"billed_units": billed, "tokens": tokens}


def rerank_results(raw: list[dict[str, JsonValue]]) -> list[JsonValue] | None:
    built: list[JsonValue] = []
    for entry in raw:
        parsed = RESULT.validate_python(entry)
        if "index" not in parsed or "relevance_score" not in parsed:
            return None
        result: dict[str, JsonValue] = {
            "index": parsed["index"],
            "relevance_score": float(parsed["relevance_score"]),
        }
        document = parsed.get("document", {})
        if document:
            result["document"] = {"text": str(document.get("text", ""))}
        built.append(result)
    return built


def rerank_response(step: RerankResponseStep) -> JsonValue:
    response = step["response"]
    raw_results = response.get("results")
    if raw_results is None:
        return {"ok": False, "error": {"kind": "no_results"}}
    results = rerank_results(RESULTS.validate_python(raw_results))
    if results is None:
        return {"ok": False, "error": {"kind": "missing_fields"}}
    usage = response.get("usage")
    usage_fields = {} if usage is None else USAGE.validate_python(usage)
    response_id = IDENTIFIER.validate_python(response.get("id"))
    return {
        "ok": True,
        "value": {
            "id": response_id if response_id else step["next_id"],
            "results": results,
            "meta": usage_blocks(usage_fields),
        },
    }


def cost_category(step: CategoryStep) -> JsonValue:
    return get_model_params_and_category(
        step["model"], CALL_TYPES[step.get("call_type", "other")]
    )


def cost_registry(step: RegistryStep) -> JsonValue:
    return has_together_registry_pricing(step["model"], litellm.model_cost)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    registered = {
        key: dict(entry) for key, entry in parsed.get("model_cost", {}).items()
    }
    with contextlib.redirect_stdout(io.StringIO()):
        litellm.register_model(registered, persist_across_reloads=False)
    return {
        "chat_supported": [
            chat_supported(step) for step in parsed.get("chat_supported", [])
        ],
        "chat_map": [chat_map(step) for step in parsed.get("chat_map", [])],
        "chat_messages": [
            chat_messages(step) for step in parsed.get("chat_messages", [])
        ],
        "completion": [completion(step) for step in parsed.get("completion", [])],
        "rerank_requests": [
            rerank_request(step) for step in parsed.get("rerank_requests", [])
        ],
        "rerank_responses": [
            rerank_response(step) for step in parsed.get("rerank_responses", [])
        ],
        "cost_categories": [
            cost_category(step) for step in parsed.get("cost_categories", [])
        ],
        "cost_registry": [
            cost_registry(step) for step in parsed.get("cost_registry", [])
        ],
    }
