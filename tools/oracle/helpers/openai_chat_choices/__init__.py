from typing import TypeAlias, TypedDict, cast

from . import choices, endpoint, reply, stream

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
PIN: str = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L588-L659"


class Step(TypedDict, total=False):
    optional_params: Object | None
    json_mode: bool
    choices: list[Json]


class ChunkStep(TypedDict):
    chunk: Object


class ScriptReply(TypedDict):
    status: int
    body: str


class ModelStep(TypedDict, total=False):
    api_base: str | None
    api_key: str | None
    env: dict[str, str]
    reply: ScriptReply


class UrlStep(TypedDict, total=False):
    api_base: str | None


class AuthStep(TypedDict, total=False):
    api_key: str | None
    headers: dict[str, str]


class ResponseStep(TypedDict):
    status: int
    raw_text: str
    raw_headers: dict[str, str]
    messages: Json
    request_data: Object


class Fields(TypedDict, total=False):
    steps: list[Json]
    api_key: str | None
    litellm_api_key: str | None
    litellm_openai_key: str | None
    api_base: str | None
    litellm_api_base: str | None
    env: dict[str, str]
    model: str


CHOICES_CASES: frozenset[str] = frozenset(
    {"tool_call_json_content", "tool_calls_finish_reason", "empty_choice"}
)
STREAM_CASES: frozenset[str] = frozenset(
    {"reasoning_content_delta", "embedded_stream_error", "stream_envelope_fields"}
)
URL_CASES: frozenset[str] = frozenset({"query_url_join"})
AUTH_CASES: frozenset[str] = frozenset({"chat_auth_content_type"})
MODEL_CASES: frozenset[str] = frozenset({"model_list", "model_list_decode_failure"})


def run(input_value: dict[str, Json]) -> Json:
    ((name, fields),) = input_value.items()
    typed = cast(Fields, fields)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L550-L898
    if name in CHOICES_CASES:
        return choices.run(cast(list[Step], typed["steps"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L588-L659
    if name in STREAM_CASES:
        return stream.run(cast(list[ChunkStep], typed["steps"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L872-L898
    if name == "response_headers":
        return reply.run(cast(list[ResponseStep], typed["steps"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L661-L708
    if name in MODEL_CASES:
        return endpoint.model_list(cast(list[ModelStep], typed["steps"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L764-L789
    if name in URL_CASES:
        return endpoint.url_join(cast(list[UrlStep], typed["steps"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L717-L743
    if name in AUTH_CASES:
        return endpoint.auth(cast(list[AuthStep], typed["steps"]))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L745-L762
    if name == "credential_accessor_defaults":
        return endpoint.credentials(typed)
    raise ValueError(f"unknown openai_chat_choices case: {name}")
