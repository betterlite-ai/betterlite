from pydantic import JsonValue

from .chat import chat_map, chat_supported, embedding_map
from .passthrough import run_passthrough
from .rerank import run_rerank
from .types import ChatMap, ChatSupported, EmbeddingMap, Input


def run_chat(input_value: ChatSupported | ChatMap | EmbeddingMap) -> JsonValue:
    match input_value["kind"]:
        case "chat_supported":
            return chat_supported(input_value)
        case "chat_map":
            return chat_map(input_value)
        case "embedding_map":
            return embedding_map(input_value)


def run(input_value: Input) -> JsonValue:
    match input_value["kind"]:
        case "chat_supported" | "chat_map" | "embedding_map":
            return run_chat(input_value)
        case (
            "rerank_select"
            | "rerank_params"
            | "rerank_headers"
            | "rerank_request"
            | "rerank_response"
        ):
            return run_rerank(input_value)
        case "passthrough_url" | "passthrough_credentials":
            return run_passthrough(input_value)
