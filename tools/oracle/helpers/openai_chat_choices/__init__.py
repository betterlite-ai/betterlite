from typing import cast

import litellm

from . import choices, endpoint, reply, stream
from .support import (
    AuthStep,
    ChunkStep,
    Fields,
    Json,
    ModelStep,
    ResponseStep,
    Step,
    UrlStep,
)

litellm.suppress_debug_info = True

CHOICES_CASES: frozenset[str] = frozenset(
    {"tool_call_json_content", "tool_calls_finish_reason", "empty_choice"}
)
STREAM_CASES: frozenset[str] = frozenset(
    {"reasoning_content_delta", "embedded_stream_error", "stream_envelope_fields"}
)
MODEL_CASES: frozenset[str] = frozenset({"model_list", "model_list_decode_failure"})


def steps_of(fields: Fields) -> list[Json]:
    steps = fields.get("steps")
    if steps is None:
        raise ValueError("openai_chat_choices needs steps")
    return steps


def run(input_value: dict[str, Json]) -> Json:
    ((name, fields),) = input_value.items()
    typed = cast(Fields, fields)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L550-L898
    if name == "credential_accessor_defaults":
        return endpoint.credentials(typed)
    steps = steps_of(typed)
    if name in CHOICES_CASES:
        result = choices.run(cast(list[Step], steps))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L588-L659
    elif name in STREAM_CASES:
        result = stream.run(cast(list[ChunkStep], steps))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L872-L898
    elif name == "response_headers":
        result = reply.run(cast(list[ResponseStep], steps))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L661-L708
    elif name in MODEL_CASES:
        result = endpoint.model_list(cast(list[ModelStep], steps))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L764-L789
    elif name == "query_url_join":
        result = endpoint.url_join(cast(list[UrlStep], steps))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L717-L743
    elif name == "chat_auth_content_type":
        result = endpoint.auth(cast(list[AuthStep], steps))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L745-L762
    else:
        raise ValueError(f"unknown openai_chat_choices case: {name}")
    return result
