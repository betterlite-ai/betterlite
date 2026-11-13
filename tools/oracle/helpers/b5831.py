import contextlib
import io
from collections.abc import Mapping
from typing import Literal, NotRequired, Protocol, TypedDict

import litellm
from litellm.llms.together_ai import cost_calculator
from litellm.llms.together_ai.chat.transformation import (
    LITELLM_INTERNAL_ASSISTANT_FIELDS as INTERNAL_FIELDS,
)
from litellm.llms.together_ai.chat.transformation import TogetherAIChatConfig
from litellm.types.llms.openai import (
    OpenAITextCompletionUserMessage as CompletionUserMessage,
)
from litellm.types.rerank import RerankRequest
from litellm.types.utils import CallTypes
from openai import APIStatusError
from pydantic import JsonValue, TypeAdapter

INPUT_MESSAGES = TypeAdapter(list[CompletionUserMessage])
OBJECT = TypeAdapter(dict[str, JsonValue])
USAGE = TypeAdapter(dict[str, int | None])
RESULTS = TypeAdapter(list[dict[str, JsonValue]])
STRING = TypeAdapter(str)
TOKENS = TypeAdapter(list[int] | list[list[int]])
CALL_TYPES: dict[str, CallTypes] = {
    "embedding": CallTypes.embedding,
    "aembedding": CallTypes.aembedding,
    "other": CallTypes.completion,
}


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


class CompletionStep(TypedDict):
    model: str
    messages: list[JsonValue]
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


class ResultFields(TypedDict, total=False):
    index: int
    relevance_score: float
    document: dict[str, JsonValue]


RESULT = TypeAdapter(ResultFields)


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
    def get_model_params_and_category(
        self, model_name: str, call_type: CallTypes
    ) -> str: ...

    def has_together_registry_pricing(
        self, model: str, cost_map: Mapping[str, JsonValue]
    ) -> bool: ...


COSTS: CostCalculator = cost_calculator


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
    stripped: list[JsonValue] = []
    for message in step["messages"]:
        if message.get("role") == "assistant":
            stripped.append(
                {key: value for key, value in message.items() if key not in INTERNAL_FIELDS}
            )
        else:
            stripped.append(message)
    return stripped


def completion(step: CompletionStep) -> JsonValue:
    messages = INPUT_MESSAGES.validate_python(step["messages"])
    if not messages:
        return {"ok": False, "error": {"kind": "integers"}}
    if len(messages) > 1:
        return {"ok": False, "error": {"kind": "multiple_prompts"}}
    content = messages[0]["content"]
    try:
        prompt: JsonValue = STRING.validate_python(content)
    except ValueError:
        try:
            TOKENS.validate_python(content)
        except ValueError:
            return {"ok": False, "error": {"kind": "multiple_prompts"}}
        return {"ok": False, "error": {"kind": "integers"}}
    return {
        "ok": True,
        "value": {"model": step["model"], "prompt": prompt, **step["optional_params"]},
    }


def rerank_request(step: RerankRequestStep) -> JsonValue:
    returned: bool | None = step.get("return_documents")
    if "return_documents" not in step:
        returned = True
    request = RerankRequest(
        model=step["model"],
        query=step["query"],
        top_n=step.get("top_n"),
        documents=step["documents"],
        rank_fields=step.get("rank_fields"),
        return_documents=returned,
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
    names = {
        "billed_units": ("search_units", "total_tokens"),
        "tokens": ("input_tokens", "output_tokens"),
    }
    return {
        block: {name: usage[name] for name in keys if name in usage}
        for block, keys in names.items()
    }


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
        document = parsed.get("document")
        if document:
            text = document.get("text")
            result["document"] = {"text": "" if text is None else str(text)}
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
    response_id = TypeAdapter(str | None).validate_python(response.get("id"))
    return {
        "ok": True,
        "value": {
            "id": response_id or step["next_id"],
            "results": results,
            "meta": usage_blocks(usage_fields),
        },
    }


def cost_category(step: CategoryStep) -> JsonValue:
    return COSTS.get_model_params_and_category(
        step["model"], CALL_TYPES[step["call_type"]]
    )


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    registered = {key: dict(entry) for key, entry in parsed["model_cost"].items()}
    with contextlib.redirect_stdout(io.StringIO()):
        litellm.register_model(registered, persist_across_reloads=False)
    return {
        "chat_supported": [
            chat_supported(step) for step in parsed["chat_supported"]
        ],
        "chat_map": [chat_map(step) for step in parsed["chat_map"]],
        "chat_messages": [
            chat_messages(step) for step in parsed["chat_messages"]
        ],
        "completion": [completion(step) for step in parsed["completion"]],
        "rerank_requests": [
            rerank_request(step) for step in parsed["rerank_requests"]
        ],
        "rerank_responses": [
            rerank_response(step) for step in parsed["rerank_responses"]
        ],
        "cost_categories": [
            cost_category(step) for step in parsed["cost_categories"]
        ],
        "cost_registry": [
            COSTS.has_together_registry_pricing(step["model"], parsed["model_cost"])
            for step in parsed["cost_registry"]
        ],
    }
