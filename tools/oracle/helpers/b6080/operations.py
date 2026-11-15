import os
from typing import Protocol

import litellm
from litellm.llms.openrouter.chat.transformation import (
    OpenRouterChatCompletionStreamingHandler,
)
from litellm.llms.openrouter.common_utils import OpenRouterException
from litellm.types.decisions import DecisionsIRResponse, DecisionsIRUsage
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue, TypeAdapter

from .configs import CHAT, DECISIONS, EMBEDDING, MODEL, RESPONSES, Params
from .inputs import (
    ChatParamsCase,
    ChunkCase,
    ChunkOut,
    DecisionsCase,
    EmbeddingCase,
    Input,
    ResponsesCase,
)
from .wire import JSON, ReplyCase, embedding_replies, requests, responses


class ChunkParser(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ModelResponseStream: ...


CHUNK_OUT: TypeAdapter[ChunkOut] = TypeAdapter(ChunkOut)


def apply_environment(names: tuple[str, ...], values: dict[str, str]) -> None:
    for name in names:
        os.environ.pop(name, None)
    for name, value in values.items():
        os.environ[name] = value


def chat_params(cases: list[ChatParamsCase]) -> JsonValue:
    mapped: list[JsonValue] = []
    for case in cases:
        optional_params: dict[str, JsonValue] = {}
        CHAT.map_openai_params(
            Params(case["requested"]), optional_params, case["model"], drop_params=False
        )
        mapped.append(optional_params)
    return JSON.validate_python(mapped)


def chunks(cases: list[ChunkCase], handler: ChunkParser) -> JsonValue:
    parsed: list[JsonValue] = []
    for case in cases:
        try:
            chunk = handler.chunk_parser(case["chunk"])
        except OpenRouterException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L43
            parsed.append({"error": {"message": error.message, "status": error.status_code}})
        else:
            parsed.append(JSON.validate_python(CHUNK_OUT.validate_json(chunk.model_dump_json())))
    return JSON.validate_python(parsed)


def embedding_answer(case: EmbeddingCase) -> JsonValue:
    apply_environment(("OR_SITE_URL", "OR_APP_NAME"), case["environment"])
    empty: dict[str, JsonValue] = {}
    headers: dict[str, str] = EMBEDDING.validate_environment(
        case["headers"], case["model"], [], empty, empty, case["api_key"], case["api_base"],
    )
    url = EMBEDDING.get_complete_url(
        case["api_base"], case["api_key"], case["model"], empty, empty
    )
    body: dict[str, JsonValue] = EMBEDDING.transform_embedding_request(
        case["model"], case["input"], case["optional_params"], headers
    )
    params: dict[str, JsonValue] = EMBEDDING.map_openai_params(
        Params(case["non_default_params"]), empty, case["model"], drop_params=False
    )
    supported: list[str] = EMBEDDING.get_supported_openai_params(case["model"])
    return {
        "supported": JSON.validate_python(supported),
        "headers": JSON.validate_python(headers),
        "url": url,
        "request": JSON.validate_python(body),
        "params": JSON.validate_python(params),
    }


def embeddings(cases: list[EmbeddingCase], replies: list[ReplyCase]) -> JsonValue:
    answers: list[JsonValue] = [embedding_answer(case) for case in cases]
    values: dict[str, JsonValue] = {
        "requests": answers,
        "replies": embedding_replies(replies),
    }
    return JSON.validate_python(values)


def response_answer(case: ResponsesCase) -> JsonValue:
    apply_environment(
        ("OPENROUTER_API_KEY", "OR_API_KEY", "OPENROUTER_API_BASE"), case["environment"]
    )
    saved_key = litellm.api_key
    saved_base = litellm.api_base
    litellm.api_key = case["global_api_key"]
    litellm.api_base = case["global_api_base"]
    empty: dict[str, JsonValue] = {}
    try:
        params = GenericLiteLLMParams(api_key=case["api_key"])
        headers: dict[str, JsonValue] = RESPONSES.validate_environment(
            case["headers"], MODEL, params
        )
        url = RESPONSES.get_complete_url(case["api_base"], empty)
    finally:
        litellm.api_key = saved_key
        litellm.api_base = saved_base
    return {
        "headers": JSON.validate_python(headers),
        "url": url,
        "provider": RESPONSES.custom_llm_provider.name.lower(),
        "websocket": RESPONSES.supports_native_websocket(),
    }


def responses_cases(cases: list[ResponsesCase]) -> JsonValue:
    answers: list[JsonValue] = [response_answer(case) for case in cases]
    return JSON.validate_python(answers)


def decisions(cases: list[DecisionsCase]) -> JsonValue:
    costs: list[JsonValue] = []
    for case in cases:
        usage = DecisionsIRUsage(extra={"cost": case["cost"]})
        response = DecisionsIRResponse(model=None, answers=(), usage=usage)
        costs.append(DECISIONS.provider_reported_cost(response))
    values: dict[str, JsonValue] = {
        "path": DECISIONS.path,
        "api_key_env": list(DECISIONS.api_key_env),
        "api_base_env": list(DECISIONS.api_base_env),
        "default_api_base": DECISIONS.get_default_api_base(),
        "complete_url": DECISIONS.get_complete_url("https://openrouter.ai/api/v1", MODEL),
        "costs": costs,
    }
    return JSON.validate_python(values)


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "chat_params":
        return chat_params(input_value["cases"])
    if input_value["op"] == "chunk":
        handler = OpenRouterChatCompletionStreamingHandler(iter([]), sync_stream=True)
        return chunks(input_value["cases"], handler)
    if input_value["op"] == "embedding":
        return embeddings(input_value["cases"], input_value["replies"])
    if input_value["op"] == "responses":
        return responses_cases(input_value["cases"])
    if input_value["op"] == "decisions":
        return decisions(input_value["cases"])
    if input_value["op"] == "request":
        return requests(input_value["cases"])
    return responses(input_value["cases"])
