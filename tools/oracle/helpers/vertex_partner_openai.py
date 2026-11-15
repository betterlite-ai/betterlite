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

class ReplyChoice(TypedDict):
    index: JsonValue
    finish_reason: NotRequired[JsonValue]
    message: dict[str, JsonValue]
class Reply(TypedDict):
    id: NotRequired[JsonValue]
    created: NotRequired[JsonValue]
    model: NotRequired[JsonValue]
    choices: list[ReplyChoice]
class ChunkChoice(TypedDict):
    index: JsonValue
    finish_reason: NotRequired[JsonValue]
    delta: dict[str, JsonValue]
    logprobs: NotRequired[JsonValue]
class Chunk(TypedDict):
    id: NotRequired[JsonValue]
    object: JsonValue
    created: NotRequired[JsonValue]
    model: NotRequired[JsonValue]
    choices: list[ChunkChoice]
    usage: NotRequired[JsonValue]
    service_tier: NotRequired[JsonValue]

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
    messages: list[dict[str, JsonValue]]
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
    handler = llama.get_model_response_iterator(
        streaming_response=iter(()), sync_stream=True, json_mode=False
    )
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_ai_partner_models/llama3/transformation.py#L73-L107
        "tuple[ParamsApi, ParamsApi, ParamsApi, ReplyApi, StreamApi]",
        (VertexAIAi21Config(), llama, VertexAIGPTOSSTransformation(), llama, handler),
    )


def mapped_params(
    api: ParamsApi, model: str, params: dict[str, JsonValue], existing: dict[str, JsonValue]
) -> dict[str, JsonValue]:
    # the partner method mutates both dicts it is given, so each case hands over copies
    return api.map_openai_params(
        non_default_params=dict(params), optional_params=dict(existing),
        model=model, drop_params=False,
    )


def token_alias(case: TokenAliasCase, ai21: ParamsApi, llama: ParamsApi) -> JsonValue:
    ai21_mapped = mapped_params(
        ai21, case["ai21_model"], case["ai21_requested"], case["ai21_optional_params"]
    )
    llama_mapped = mapped_params(
        llama, case["llama_model"], case["llama_requested"], case["llama_optional_params"]
    )
    return JSON.validate_python(
        {"ai21_optional_params": ai21_mapped, "llama_optional_params": llama_mapped}
    )


def exclusions(case: ExclusionsCase, llama: ParamsApi) -> JsonValue:
    mapped = mapped_params(
        llama, case["deployed_model"], case["requested"], case["optional_params"]
    )
    return JSON.validate_python(
        {
            "publisher_params": llama.get_supported_openai_params(case["publisher_model"]),
            "deployed_params": llama.get_supported_openai_params(case["deployed_model"]),
            "deployed_mapped": mapped,
        }
    )


def gpt_oss(case: GptOssCase, api: ParamsApi) -> JsonValue:
    model_mapped = mapped_params(api, case["model"], case["requested"], case["optional_params"])
    funded_mapped = mapped_params(
        api, case["funded_model"], case["requested"], case["optional_params"]
    )
    return JSON.validate_python(
        {
            "model_params": api.get_supported_openai_params(case["model"]),
            "model_mapped": model_mapped,
            "funded_params": api.get_supported_openai_params(case["funded_model"]),
            "funded_mapped": funded_mapped,
        }
    )


def logging_object() -> Logging:
    return Logging(
        model="meta/llama-3.1-405b-instruct-maas", messages=None,
        stream=False, call_type="completion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b1335", function_id="b1335",
    )


def reply_value(api: ReplyApi, requested_model: str, source: dict[str, JsonValue]) -> JsonValue:
    args: ResponseArgs = {
        "model": requested_model,
        "raw_response": httpx.Response(200, json=source),
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
    body = choice["message"]
    return JSON.validate_python(
        {
            "index": choice["index"],
            "finish_reason": choice.get("finish_reason"),
            "message": {"role": body["role"], "content": body.get("content"),
                        "tool_calls": body.get("tool_calls")},
        }
    )


def next_chunk(api: StreamApi) -> ModelResponseStream | None:
    try:
        return next(api)
    except StopIteration:
        return None


def choice_value(choice: ChunkChoice) -> JsonValue:
    delta = choice["delta"]
    fields = ("role", "content", "function_call", "tool_calls", "audio")
    delta_value: dict[str, JsonValue] = {name: delta.get(name) for name in fields}
    for name in ("reasoning_content", "provider_specific_fields"):
        value = delta.get(name)
        if value is not None:
            delta_value[name] = value
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
    reply_case = input_value["missing_reply_defaults"]
    return JSON.validate_python(
        {
            "token_alias_overwrite": token_alias(input_value["token_alias_overwrite"], ai21, llama),
            "self_deployed_exclusions": exclusions(input_value["self_deployed_exclusions"], llama),
            "gpt_oss_singular_tool": gpt_oss(input_value["gpt_oss_singular_tool"], gpt_oss_api),
            "missing_reply_defaults": {
                "defaulted": reply_value(
                    reply_api, reply_case["requested_model"], reply_case["reply"]
                ),
                "named": reply_value(
                    reply_api, reply_case["requested_model"], reply_case["named_reply"]
                ),
            },
            "initial_missing_role": streamed(input_value["initial_missing_role"]),
            "initial_terminal_split": streamed(input_value["initial_terminal_split"]),
            "empty_initial_delta": streamed(input_value["empty_initial_delta"]),
        }
    )
