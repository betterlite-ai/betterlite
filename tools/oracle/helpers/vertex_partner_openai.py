import json
from datetime import UTC, datetime
from typing import Final, NotRequired, Protocol, TypedDict, Unpack, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.vertex_ai_partner_models.ai21.transformation import (
    VertexAIAi21Config,
)
from litellm.llms.vertex_ai.vertex_ai_partner_models.gpt_oss.transformation import (
    VertexAIGPTOSSTransformation,
)
from litellm.llms.vertex_ai.vertex_ai_partner_models.llama3.transformation import (
    VertexAILlama3Config,
)
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import JsonValue, TypeAdapter

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


class TokenAliasCase(TypedDict):
    ai21_model: str
    ai21_requested: dict[str, JsonValue]
    ai21_optional_params: dict[str, JsonValue]
    llama_model: str
    llama_requested: dict[str, JsonValue]
    llama_optional_params: dict[str, JsonValue]


class ExclusionsCase(TypedDict):
    publisher_model: str
    deployed_model: str
    requested: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class GptOssCase(TypedDict):
    model: str
    funded_model: str
    requested: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class ReplyCase(TypedDict):
    requested_model: str
    reply: dict[str, JsonValue]
    named_reply: dict[str, JsonValue]


class StreamCase(TypedDict):
    chunks: list[dict[str, JsonValue]]


class Input(TypedDict):
    token_alias_overwrite: TokenAliasCase
    self_deployed_exclusions: ExclusionsCase
    gpt_oss_singular_tool: GptOssCase
    missing_reply_defaults: ReplyCase
    initial_missing_role: StreamCase
    initial_terminal_split: StreamCase
    empty_initial_delta: StreamCase


class Message(TypedDict, total=False):
    role: str
    content: JsonValue


class ReplyMessage(TypedDict):
    role: str
    content: NotRequired[str | None]
    tool_calls: NotRequired[list[JsonValue] | None]


class ReplyChoice(TypedDict):
    index: int
    finish_reason: NotRequired[str | None]
    message: ReplyMessage


class Reply(TypedDict):
    id: NotRequired[str | None]
    created: NotRequired[int | None]
    model: NotRequired[str | None]
    choices: list[ReplyChoice]


class ChunkDelta(TypedDict):
    role: NotRequired[str | None]
    content: NotRequired[str | None]
    function_call: NotRequired[JsonValue]
    tool_calls: NotRequired[JsonValue]
    audio: NotRequired[JsonValue]
    reasoning_content: NotRequired[str | None]
    provider_specific_fields: NotRequired[dict[str, JsonValue] | None]


class ChunkChoice(TypedDict):
    index: int
    finish_reason: NotRequired[str | None]
    delta: ChunkDelta
    logprobs: NotRequired[JsonValue]


class Chunk(TypedDict):
    id: NotRequired[str | None]
    object: str
    created: NotRequired[int | None]
    model: NotRequired[str | None]
    choices: list[ChunkChoice]
    usage: NotRequired[JsonValue]
    service_tier: NotRequired[str | None]


class ParamsApi(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


class ResponseArgs(TypedDict):
    model: str
    raw_response: httpx.Response
    model_response: ModelResponse
    logging_obj: Logging
    request_data: dict[str, JsonValue]
    messages: list[Message]
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    encoding: None


class ReplyApi(Protocol):
    def transform_response(self, **kwargs: Unpack[ResponseArgs]) -> ModelResponse: ...


class StreamApi(Protocol):
    def chunk_parser(self, chunk: dict[str, JsonValue]) -> ModelResponseStream: ...

    def __next__(self) -> ModelResponseStream: ...


LITELLM_PARAMS: Final[dict[str, JsonValue]] = {}
JSON: Final = TypeAdapter(JsonValue)


def apis() -> tuple[ParamsApi, ParamsApi, ParamsApi, ReplyApi, StreamApi]:
    llama = VertexAILlama3Config()
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_ai_partner_models/llama3/transformation.py#L73-L107
        "tuple[ParamsApi, ParamsApi, ParamsApi, ReplyApi, StreamApi]",
        (
            VertexAIAi21Config(),
            llama,
            VertexAIGPTOSSTransformation(),
            llama,
            llama.get_model_response_iterator(
                streaming_response=iter(()), sync_stream=True, json_mode=False
            ),
        ),
    )


def mapped_params(
    api: ParamsApi,
    model: str,
    requested: dict[str, JsonValue],
    optional_params: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    # the partner method mutates both dicts it is given, so each case hands over copies
    return api.map_openai_params(
        non_default_params=dict(requested),
        optional_params=dict(optional_params),
        model=model,
        drop_params=False,
    )


def token_alias(case: TokenAliasCase, ai21: ParamsApi, llama: ParamsApi) -> JsonValue:
    return JSON.validate_python(
        {
            "ai21_optional_params": mapped_params(
                ai21,
                case["ai21_model"],
                case["ai21_requested"],
                case["ai21_optional_params"],
            ),
            "llama_optional_params": mapped_params(
                llama,
                case["llama_model"],
                case["llama_requested"],
                case["llama_optional_params"],
            ),
        }
    )


def exclusions(case: ExclusionsCase, llama: ParamsApi) -> JsonValue:
    return JSON.validate_python(
        {
            "publisher_params": llama.get_supported_openai_params(
                model=case["publisher_model"]
            ),
            "deployed_params": llama.get_supported_openai_params(
                model=case["deployed_model"]
            ),
            "deployed_mapped": mapped_params(
                llama, case["deployed_model"], case["requested"], case["optional_params"]
            ),
        }
    )


def gpt_oss(case: GptOssCase, api: ParamsApi) -> JsonValue:
    return JSON.validate_python(
        {
            "model_params": api.get_supported_openai_params(model=case["model"]),
            "model_mapped": mapped_params(
                api, case["model"], case["requested"], case["optional_params"]
            ),
            "funded_params": api.get_supported_openai_params(model=case["funded_model"]),
            "funded_mapped": mapped_params(
                api, case["funded_model"], case["requested"], case["optional_params"]
            ),
        }
    )


def logging_object() -> Logging:
    return Logging(
        model="meta/llama-3.1-405b-instruct-maas",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b1335",
        function_id="b1335",
    )


def reply_value(
    api: ReplyApi, requested_model: str, source: dict[str, JsonValue]
) -> JsonValue:
    args: ResponseArgs = {
        "model": requested_model,
        "raw_response": httpx.Response(200, text=json.dumps(source)),
        "model_response": ModelResponse(),
        "logging_obj": logging_object(),
        "request_data": {},
        "messages": [],
        "optional_params": {},
        "litellm_params": LITELLM_PARAMS,
        "encoding": None,
    }
    result = api.transform_response(**args)
    view = TypeAdapter(Reply).validate_json(result.model_dump_json())
    return JSON.validate_python(
        {
            "id": view.get("id"),
            "created": view.get("created"),
            "model": view.get("model"),
            "choices": [reply_choice(choice) for choice in view["choices"]],
        }
    )


def reply_choice(choice: ReplyChoice) -> JsonValue:
    message = choice["message"]
    return JSON.validate_python(
        {
            "index": choice["index"],
            "finish_reason": choice.get("finish_reason"),
            "message": {
                "role": message["role"],
                "content": message.get("content"),
                "tool_calls": message.get("tool_calls"),
            },
        }
    )


# LiteLLM converts the provider's usage into its own Usage model, which the shared reply builder
# owns; this comparison covers the identity fields and the transformed choices.
def reply(case: ReplyCase, api: ReplyApi) -> JsonValue:
    return JSON.validate_python(
        {
            "defaulted": reply_value(api, case["requested_model"], case["reply"]),
            "named": reply_value(api, case["requested_model"], case["named_reply"]),
        }
    )


def next_chunk(api: StreamApi) -> ModelResponseStream | None:
    queued: ModelResponseStream | None
    try:
        queued = next(api)
    except StopIteration:
        queued = None
    return queued


def choice_value(choice: ChunkChoice) -> JsonValue:
    delta = choice["delta"]
    delta_value: dict[str, JsonValue] = {
        "role": delta.get("role"),
        "content": delta.get("content"),
        "function_call": delta.get("function_call"),
        "tool_calls": delta.get("tool_calls"),
        "audio": delta.get("audio"),
    }
    reasoning = delta.get("reasoning_content")
    if reasoning is not None:
        delta_value["reasoning_content"] = reasoning
    provider_fields = delta.get("provider_specific_fields")
    if provider_fields is not None:
        delta_value["provider_specific_fields"] = provider_fields
    return JSON.validate_python(
        {
            "index": choice["index"],
            "finish_reason": choice.get("finish_reason"),
            "delta": delta_value,
            "logprobs": choice.get("logprobs"),
        }
    )


def chunk_value(chunk: ModelResponseStream) -> JsonValue:
    view = TypeAdapter(Chunk).validate_json(chunk.model_dump_json())
    entry: dict[str, JsonValue] = {
        "id": view.get("id"),
        "object": view["object"],
        "created": view.get("created"),
        "model": view.get("model"),
        "choices": [choice_value(choice) for choice in view["choices"]],
    }
    usage = view.get("usage")
    if usage is not None:
        entry["usage"] = usage
    tier = view.get("service_tier")
    if tier is not None:
        entry["service_tier"] = tier
    return JSON.validate_python(entry)


# get_model_response_iterator gives each stream its own handler, so each case asks for a fresh one.
def streamed(case: StreamCase) -> JsonValue:
    api = apis()[4]
    emitted: list[JsonValue] = []
    for source in case["chunks"]:
        emitted.append(chunk_value(api.chunk_parser(source)))
        queued = next_chunk(api)
        if queued is not None:
            emitted.append(chunk_value(queued))
    return JSON.validate_python(emitted)


def run(input_value: Input) -> JsonValue:
    ai21, llama, gpt_oss_api, reply_api, _ = apis()
    return JSON.validate_python(
        {
            "token_alias_overwrite": token_alias(
                input_value["token_alias_overwrite"], ai21, llama
            ),
            "self_deployed_exclusions": exclusions(
                input_value["self_deployed_exclusions"], llama
            ),
            "gpt_oss_singular_tool": gpt_oss(
                input_value["gpt_oss_singular_tool"], gpt_oss_api
            ),
            "missing_reply_defaults": reply(
                input_value["missing_reply_defaults"], reply_api
            ),
            "initial_missing_role": streamed(input_value["initial_missing_role"]),
            "initial_terminal_split": streamed(input_value["initial_terminal_split"]),
            "empty_initial_delta": streamed(input_value["empty_initial_delta"]),
        }
    )
