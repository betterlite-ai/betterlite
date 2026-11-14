from typing import TypeAlias, TypedDict, cast

import litellm
from litellm.llms.snowflake.chat.transformation import SnowflakeConfig
from litellm.llms.snowflake.embedding.transformation import SnowflakeEmbeddingConfig
from pydantic import JsonValue

from .chat import (
    ChunkStep,
    EmbeddingRequestStep,
    EmbeddingResponseStep,
    EventStep,
    RequestStep,
    ResponseStep,
    chunk_step,
    embedding_response_step,
    event_step,
    response_step,
)
from .config import (
    HeadersStep,
    InfoStep,
    ParamsStep,
    UrlStep,
    headers_step,
    info_step,
    params_step,
    url_step,
)
from .views import ChatView, EmbeddingView, Pair, StreamHandler

__all__ = ["litellm", "run"]

Step: TypeAlias = (
    UrlStep
    | HeadersStep
    | ParamsStep
    | InfoStep
    | RequestStep
    | ResponseStep
    | EmbeddingRequestStep
    | EmbeddingResponseStep
    | ChunkStep
    | EventStep
)


class Input(TypedDict):
    steps: list[Step]


def run_step(chat: ChatView, embed: EmbeddingView, handler: StreamHandler, step: Step) -> JsonValue:
    result: JsonValue
    if step["kind"] == "url":
        result = url_step(chat, embed, step)
    elif step["kind"] == "headers":
        result = headers_step(chat, step)
    elif step["kind"] == "params":
        result = params_step(chat, embed, step)
    elif step["kind"] == "info":
        result = info_step(step)
    elif step["kind"] == "request":
        result = {
            "body": chat.transform_request(
                model=step["model"],
                messages=step["messages"],
                optional_params=dict(step["options"]),
                litellm_params={},
                headers={},
            )
        }
    elif step["kind"] == "response":
        result = response_step(chat, step)
    elif step["kind"] == "embedding_request":
        result = {
            "body": embed.transform_embedding_request(
                model=step["model"],
                input=step["text"],
                optional_params=dict(step["options"]),
                headers={},
            )
        }
    elif step["kind"] == "embedding_response":
        result = embedding_response_step(embed, step)
    elif step["kind"] == "chunk":
        result = chunk_step(handler, step["chunk"])
    else:
        result = event_step(handler, step["chunk"])
    return result


def run(input_value: Input) -> JsonValue:
    chat, embed = cast("Pair", (SnowflakeConfig(), SnowflakeEmbeddingConfig()))  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L69
    handler = chat.get_model_response_iterator(
        streaming_response=None, sync_stream=False, json_mode=False
    )
    return {"steps": [run_step(chat, embed, handler, step) for step in input_value["steps"]]}
