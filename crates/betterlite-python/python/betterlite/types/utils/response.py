import time
import uuid
from collections.abc import Mapping, Sequence
from typing import Final, Unpack

from pydantic import JsonValue, PrivateAttr
from typing_extensions import override

from betterlite.types.utils.base import (
    DictJsonObject,
    Dumpable,
    DumpOptions,
    InitValue,
    ModelValue,
    ResponseObject,
    validated_value,
)
from betterlite.types.utils.choices import Choices, FinishReason, StreamingChoices
from betterlite.types.utils.tool_calls import ChatCompletionTokenLogprob
from betterlite.types.utils.usage import Usage

OPENAI_RESPONSE_HEADERS: Final[tuple[str, ...]] = (
    "x-ratelimit-remaining-requests", "x-ratelimit-remaining-tokens",
    "x-ratelimit-limit-requests", "x-ratelimit-limit-tokens",
    "x-ratelimit-reset-requests", "x-ratelimit-reset-tokens",
)


def process_response_headers(headers: Mapping[str, str]) -> dict[str, JsonValue]:
    openai_headers: dict[str, JsonValue] = {}
    processed_headers: dict[str, JsonValue] = {}
    additional_headers: dict[str, JsonValue] = {}
    for key, value in headers.items():
        if key in OPENAI_RESPONSE_HEADERS:
            openai_headers[key] = value
        if key.startswith("llm_provider-"):
            processed_headers[key] = value
        else:
            additional_headers[f"llm_provider-{key}"] = value
    return {**openai_headers, **processed_headers, **additional_headers}


def _generate_id() -> str:
    generated = uuid.uuid4()
    return "chatcmpl-" + str(generated)


class ModelResponseBase(ResponseObject):
    id: str
    created: int
    model: str | None = None
    object: str
    system_fingerprint: str | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)
    _response_headers: dict[str, JsonValue] | None = PrivateAttr(default=None)

    @override
    def __init__(self, **data: InitValue) -> None:
        super().__init__(**data)

    def set_provider_response_headers(self, headers: Mapping[str, str]) -> None:
        self._hidden_params["additional_headers"] = process_response_headers(headers)

    @override
    def model_dump(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        mode = kwargs.get("mode")
        exclude_none = kwargs.get("exclude_none")
        exclude_unset = kwargs.get("exclude_unset")
        if exclude_unset is None and exclude_none is None:
            exclude_unset = True
        warnings = kwargs.get("warnings")
        dumped: dict[str, JsonValue] = super().model_dump(
            mode="python" if mode is None else mode,
            include=kwargs.get("include"), exclude=kwargs.get("exclude"),
            context=kwargs.get("context"), by_alias=kwargs.get("by_alias"),
            exclude_unset=exclude_unset is True,
            exclude_defaults=kwargs.get("exclude_defaults") is True,
            exclude_none=exclude_none is True,
            exclude_computed_fields=kwargs.get("exclude_computed_fields") is True,
            round_trip=kwargs.get("round_trip") is True,
            warnings=True if warnings is None else warnings,
            fallback=kwargs.get("fallback"),
            serialize_as_any=kwargs.get("serialize_as_any") is True,
            polymorphic_serialization=kwargs.get("polymorphic_serialization"),
        )
        return dumped


class ModelResponse(ModelResponseBase, DictJsonObject):
    choices: list[Choices]

    @override
    def __init__(
        self, id: JsonValue = None, choices: Sequence[ModelValue] | None = None,
        created: JsonValue = None, model: JsonValue = None, object: JsonValue = None,
        system_fingerprint: JsonValue = None, usage: ModelValue = None,
        stream: JsonValue = None, stream_options: JsonValue = None,
        response_ms: JsonValue = None, hidden_params: Mapping[str, JsonValue] | None = None,
        _response_headers: Mapping[str, JsonValue] | None = None, **params: InitValue,
    ) -> None:
        selected: list[Choices] = (
            [Choices()] if choices is None
            else [validated_value(choice, Choices) for choice in choices]
        )
        extras: dict[str, InitValue] = dict(params)
        if usage is not None:
            extras["usage"] = response_usage(usage)
        super().__init__(
            id=_generate_id() if id is None else id, choices=selected,
            created=int(time.time()) if created is None else created, model=model,
            object="chat.completion", system_fingerprint=system_fingerprint, **extras,
        )
        if hidden_params is not None:
            self._hidden_params = dict(hidden_params)
        if _response_headers is not None:
            self._response_headers = dict(_response_headers)


class ModelResponseStream(ModelResponseBase, DictJsonObject):
    choices: list[StreamingChoices]
    provider_specific_fields: dict[str, JsonValue] | None = None

    @override
    def __init__(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2107-L2159
        self, choices: ModelValue | Sequence[ModelValue] = None, id: JsonValue = None,
        created: JsonValue = None, provider_specific_fields: Mapping[str, JsonValue] | None = None,
        **kwargs: InitValue,
    ) -> None:
        extras: dict[str, InitValue] = dict(kwargs)
        extras["object"] = "chat.completion.chunk"
        if "usage" in kwargs and kwargs["usage"] is not None:
            extras["usage"] = response_usage(kwargs["usage"])
        selected: list[StreamingChoices] = [StreamingChoices()]
        if choices is not None:
            match choices:
                case list():
                    selected = [
                        validated_value(choice, StreamingChoices) for choice in choices
                    ]
                case bool() | int() | float() | str() | Mapping() | Dumpable() | Sequence():
                    selected = [StreamingChoices()]
        super().__init__(
            choices=selected, id=_generate_id() if id is None else id,
            created=int(time.time()) if created is None else created,
            provider_specific_fields=provider_specific_fields, **extras,
        )


class StreamingChunkDelta(ResponseObject):
    content: str | None = None
    function_call: JsonValue = None
    refusal: str | None = None
    role: str | None = None
    tool_calls: JsonValue = None


class ChunkLogprobs(ResponseObject):
    content: list[ChatCompletionTokenLogprob] | None = None
    refusal: list[ChatCompletionTokenLogprob] | None = None


class StreamingChunkChoice(ResponseObject):
    delta: StreamingChunkDelta
    finish_reason: FinishReason | None = None
    index: int = 0
    logprobs: ChunkLogprobs | None = None


class ChunkCompletionTokensDetails(ResponseObject):
    accepted_prediction_tokens: int | None = None
    audio_tokens: int | None = None
    reasoning_tokens: int | None = None
    rejected_prediction_tokens: int | None = None


class ChunkPromptTokensDetails(ResponseObject):
    audio_tokens: int | None = None
    cache_write_tokens: int | None = None
    cached_tokens: int | None = None


class ChunkUsage(ResponseObject):
    completion_tokens: int
    prompt_tokens: int
    total_tokens: int
    completion_tokens_details: ChunkCompletionTokensDetails | None = None
    prompt_tokens_details: ChunkPromptTokensDetails | None = None


class StreamingChatCompletionChunk(ResponseObject):
    id: str
    choices: list[StreamingChunkChoice]
    created: int
    model: str
    object: str
    moderation: JsonValue = None
    service_tier: str | None = None
    system_fingerprint: str | None = None
    usage: ChunkUsage | None = None

    @override
    def __init__(self, **kwargs: InitValue) -> None:
        kwargs["choices"] = chunk_choices(kwargs["choices"])
        super().__init__(**kwargs)


def chunk_choices(raw_choices: InitValue) -> InitValue:
    if raw_choices is None:
        return None
    match raw_choices:
        case Sequence():
            return [chunk_choice_value(choice) for choice in raw_choices]
        case Dumpable() | bool() | int() | float() | Mapping():
            return raw_choices


def chunk_choice_value(choice: InitValue) -> InitValue:
    if choice is None:
        return None
    match choice:
        case Mapping():
            dump: dict[str, JsonValue] = StreamingChoices.model_validate(choice).model_dump()
            return StreamingChunkChoice.model_validate(dump)
        case Dumpable():
            dumped: dict[str, JsonValue] = choice.model_dump()
            return chunk_choice_value(dumped)
        case bool() | int() | float() | Sequence():
            return choice


def response_usage(value: InitValue) -> InitValue:
    if value is None:
        return None
    match value:
        case Usage():
            return value
        case Dumpable():
            dumped: dict[str, JsonValue] = value.model_dump()
            return Usage.model_validate(dumped)
        case Mapping():
            return Usage.model_validate(value)
        case bool() | int() | float() | Sequence():
            return value


__all__ = [
    "OPENAI_RESPONSE_HEADERS", "ModelResponse", "ModelResponseBase", "ModelResponseStream",
    "StreamingChatCompletionChunk", "_generate_id", "process_response_headers",
]
