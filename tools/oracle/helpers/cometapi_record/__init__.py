import litellm
from litellm.llms.cometapi.chat.transformation import (
    CometAPIChatCompletionStreamingHandler as CometStreamHandler,
)
from litellm.llms.cometapi.chat.transformation import CometAPIConfig
from litellm.llms.cometapi.embed.transformation import CometAPIEmbeddingConfig
from pydantic import JsonValue

from .chat_steps import chunk, options, request, url
from .embedding_steps import (
    embedding_headers,
    embedding_options,
    embedding_reply,
    embedding_request,
    embedding_url,
)
from .inputs import Input

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    chat = CometAPIConfig()
    embed = CometAPIEmbeddingConfig()
    handler = CometStreamHandler(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_model_iterator.py#L67-L71
        streaming_response=iter([""]), sync_stream=False, json_mode=False
    )
    results: list[JsonValue] = []
    for step in input_value["steps"]:
        if step["kind"] == "options":
            results.append(options(chat, step))
        elif step["kind"] == "request":
            results.append(request(chat, step))
        elif step["kind"] == "url":
            results.append(url(chat, step))
        elif step["kind"] == "chunk":
            results.append(chunk(handler, step))
        elif step["kind"] == "embedding_headers":
            results.append(embedding_headers(embed, step))
        elif step["kind"] == "embedding_url":
            results.append(embedding_url(embed, step))
        elif step["kind"] == "embedding_options":
            results.append(embedding_options(embed, step))
        elif step["kind"] == "embedding_request":
            results.append(embedding_request(embed, step))
        else:
            results.append(embedding_reply(embed, step))
    return results
