from dataclasses import dataclass, field
from functools import partial
from operator import is_
from types import SimpleNamespace
from typing import Annotated, Final, Protocol, TypedDict, cast

import httpx
from litellm._uuid import uuid as fastuuid
from litellm.llms.vertex_ai.gemini import vertex_and_google_ai_studio_gemini as gemini
from litellm.llms.vertex_ai.gemini.vertex_and_google_ai_studio_gemini import VertexGeminiConfig
from litellm.types import utils as litellm_utils
from litellm.types.utils import ModelResponse, ModelResponseStream
from pydantic import AliasPath, Field, Json, JsonValue, TypeAdapter

JSON_VALUE: Final = TypeAdapter(JsonValue)
UNSET: Final = partial(is_, None)


class Input(TypedDict):
    streaming: bool
    legacy_function: bool
    cumulative_tool_call_idx: int
    now: int
    generated_id: str
    body: dict[str, JsonValue]


class StreamConfig(Protocol):
    # ast-grep-ignore: no-long-names
    def _check_prompt_level_content_filter(
        self, processed_chunk: dict[str, JsonValue], response_id: str | None
    ) -> ModelResponseStream | None: ...

    def _process_candidates(
        self,
        _candidates: list[JsonValue],
        model_response: ModelResponseStream,
        standard_optional_params: dict[str, JsonValue],
        cumulative_tool_call_index: int,
    ) -> tuple[list[JsonValue], list[JsonValue], list[JsonValue], list[JsonValue], int]: ...


STREAM_CONFIG: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L2189
    StreamConfig, VertexGeminiConfig()
)


@dataclass
class Body:
    candidates: list[JsonValue] = field(default_factory=list)


@dataclass
class Function:
    name: str
    arguments: Json[JsonValue]
    provider_specific_fields: Annotated[JsonValue, Field(exclude_if=UNSET)] = None


@dataclass
class ToolCall:
    id: str
    type: str
    index: int
    function: Function
    provider_specific_fields: Annotated[JsonValue, Field(exclude_if=UNSET)] = None


@dataclass
class Token:
    token: str
    logprob: float
    top_logprobs: Annotated[list["Token"] | None, Field(exclude_if=UNSET)] = None


@dataclass
class Logprobs:
    content: list[Token] | None = None


@dataclass
class Delta:
    role: str | None = None
    content: str | None = None
    reasoning_content: str | None = None
    tool_calls: list[ToolCall] | None = None
    function_call: Function | None = None
    images: JsonValue = None
    annotations: JsonValue = None
    provider_specific_fields: JsonValue = None


@dataclass
class Message(Delta):
    thinking_blocks: JsonValue = None
    audio: JsonValue = None


@dataclass
class Complete:
    index: int
    finish_reason: str
    message: Message
    logprobs: Logprobs | None = None
    native_finish_reason: Annotated[
        str | None,
        Field(validation_alias=AliasPath("provider_specific_fields", "native_finish_reason")),
    ] = None


@dataclass
class Streamed:
    index: int
    finish_reason: str
    delta: Delta
    logprobs: Logprobs | None = None


@dataclass
class Usage:
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: JsonValue = None
    completion_tokens_details: JsonValue = None


@dataclass
class Response:
    choices: list[Complete]
    usage: Usage
    grounding_metadata: Annotated[
        JsonValue, Field(validation_alias="vertex_ai_grounding_metadata")
    ] = None
    url_context_metadata: Annotated[
        JsonValue, Field(validation_alias="vertex_ai_url_context_metadata")
    ] = None
    safety_ratings: Annotated[JsonValue, Field(validation_alias="vertex_ai_safety_results")] = None
    citation_metadata: Annotated[
        JsonValue, Field(validation_alias="vertex_ai_citation_metadata")
    ] = None


BODY: Final = TypeAdapter(Body)
RESPONSE: Final = TypeAdapter(Response)
COMPLETE: Final = TypeAdapter(Complete)
USAGE: Final = TypeAdapter(Usage)
STREAMED: Final = TypeAdapter(Streamed)


def _inject(input_value: Input) -> None:
    fixed = SimpleNamespace(
        uuid4=partial(fastuuid.UUID, hex=input_value["generated_id"]),
        time=partial(float, input_value["now"]),
    )
    gemini.uuid = fixed  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAttributeAccessIssue, reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1581
    litellm_utils.uuid = fixed  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAttributeAccessIssue, reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1358
    gemini.time = fixed  # type: ignore[attr-defined,assignment]  # pyright: ignore[reportAttributeAccessIssue, reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1524


def _complete(input_value: Input, optional_params: dict[str, JsonValue]) -> JsonValue:
    raw = VertexGeminiConfig()._transform_google_generate_content_to_openai_model_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L2412
        input_value["body"],
        ModelResponse(),
        "gemini-2.5-flash",
        cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L109
            "gemini.LoggingClass", SimpleNamespace(optional_params=optional_params)
        ),
        httpx.Response(200),
    )
    response = RESPONSE.validate_json(raw.model_dump_json())
    choices = [
        JSON_VALUE.validate_json(COMPLETE.dump_json(choice, round_trip=True))
        for choice in response.choices
    ]
    if response.grounding_metadata is None:
        usage = JSON_VALUE.validate_json(USAGE.dump_json(response.usage))
        return {"choices": choices, "usage": usage, "metadata": None}
    metadata = {
        "grounding_metadata": response.grounding_metadata,
        "url_context_metadata": response.url_context_metadata,
        "safety_ratings": response.safety_ratings,
        "citation_metadata": response.citation_metadata,
    }
    return {"choices": choices, "usage": None, "metadata": metadata}


def _streamed(input_value: Input, optional_params: dict[str, JsonValue]) -> JsonValue:
    body = input_value["body"]
    blocked = STREAM_CONFIG._check_prompt_level_content_filter(body, None)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L1936
    chunk = ModelResponseStream(choices=[]) if blocked is None else blocked
    next_index = input_value["cumulative_tool_call_idx"]
    metadata: JsonValue = None
    if blocked is None:
        processed = STREAM_CONFIG._process_candidates(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/vertex_and_google_ai_studio_gemini.py#L2189
            BODY.validate_python(body).candidates, chunk, optional_params, next_index
        )
        metadata = {
            "grounding_metadata": processed[0],
            "url_context_metadata": processed[1],
            "safety_ratings": processed[2],
            "citation_metadata": processed[3],
        }
        next_index = processed[4]
    choices = [
        JSON_VALUE.validate_json(
            STREAMED.dump_json(STREAMED.validate_json(choice.model_dump_json()), round_trip=True)
        )
        for choice in chunk.choices
    ]
    return {"choices": choices, "metadata": metadata, "next_tool_call_index": next_index}


def run(input_value: Input) -> JsonValue:
    _inject(input_value)
    optional_params: dict[str, JsonValue] = (
        {"functions": [{}]} if input_value["legacy_function"] else {}
    )
    if input_value["streaming"]:
        return _streamed(input_value, optional_params)
    return _complete(input_value, optional_params)
