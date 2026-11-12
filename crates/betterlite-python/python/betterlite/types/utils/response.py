import time
import uuid
from collections.abc import Mapping, Sequence
from typing import ClassVar, Final, Literal

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.choices import Choices, FinishReason, StreamingChoices
from betterlite.types.utils.usage import Usage
from betterlite.types.utils.values import JsonValue, StoredValue, ValidationError

OPENAI_RESPONSE_HEADERS: Final[tuple[str, ...]] = (
    "x-ratelimit-remaining-requests", "x-ratelimit-remaining-tokens",
    "x-ratelimit-limit-requests", "x-ratelimit-limit-tokens",
    "x-ratelimit-reset-requests", "x-ratelimit-reset-tokens",
)


def process_response_headers(headers: Mapping[str, str]) -> dict[str, str]:
    openai_headers: dict[str, str] = {}
    processed_headers: dict[str, str] = {}
    additional_headers: dict[str, str] = {}
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
    model: str | None
    object: str
    system_fingerprint: str | None
    _hidden_params: dict[str, StoredValue]
    _response_headers: dict[str, StoredValue] | None
    defaults: ClassVar[dict[str, StoredValue]] = {"_hidden_params": {}}

    def set_provider_response_headers(self, headers: Mapping[str, str]) -> None:
        self._hidden_params["additional_headers"] = process_response_headers(headers)

    @override
    def model_dump(
        self, *, mode: Literal["python", "json"] = "python",
        exclude_unset: bool | None = None, exclude_none: bool | None = None,
    ) -> JsonValue:
        if exclude_unset is None and exclude_none is None:
            return super().model_dump(mode=mode, exclude_unset=True)
        return super().model_dump(
            mode=mode, exclude_unset=exclude_unset is True, exclude_none=exclude_none is True,
        )


class ModelResponse(ModelResponseBase):
    choices: list[Choices]

    @override
    def __init__(
        self, id: str | None = None, choices: StoredValue = None, created: int | None = None,
        model: str | None = None, object: StoredValue = None,
        system_fingerprint: str | None = None, usage: StoredValue = None,
        stream: bool | None = None, stream_options: StoredValue = None,
        response_ms: float | None = None, hidden_params: dict[str, StoredValue] | None = None,
        _response_headers: dict[str, StoredValue] | None = None, **params: StoredValue,
    ) -> None:
        data: dict[str, StoredValue] = dict(params)
        match choices:
            case list():
                data["choices"] = choices
            case _other:
                data["choices"] = [Choices()]
        data["id"] = _generate_id() if id is None else id
        data["created"] = int(time.time()) if created is None else created
        data.update(model=model, object="chat.completion", system_fingerprint=system_fingerprint)
        if usage is not None:
            data["usage"] = response_usage(usage)
        if hidden_params is not None:
            data["_hidden_params"] = hidden_params
        if _response_headers is not None:
            data["_response_headers"] = _response_headers
        super().__init__(**data)

    @override
    def json(self, **_kwargs: JsonValue) -> JsonValue:
        return self.model_dump()


class ModelResponseStream(ModelResponseBase):
    choices: list[StreamingChoices]
    provider_specific_fields: dict[str, JsonValue] | None

    @override
    def __init__(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2107-L2159
        self, choices: StoredValue = None, id: str | None = None,
        created: int | None = None, provider_specific_fields: StoredValue = None,
        **kwargs: StoredValue,
    ) -> None:
        data: dict[str, StoredValue] = dict(kwargs)
        match choices:
            case list():
                data["choices"] = choices
            case _other:
                data["choices"] = [StreamingChoices()]
        data["id"] = _generate_id() if id is None else id
        data["created"] = int(time.time()) if created is None else created
        data.update(
            object="chat.completion.chunk", provider_specific_fields=provider_specific_fields,
        )
        if "usage" in kwargs and kwargs["usage"] is not None:
            data["usage"] = response_usage(kwargs["usage"])
        super().__init__(**data)

    @override
    def json(self, **_kwargs: JsonValue) -> JsonValue:
        return self.model_dump()


class StreamingChunkDelta(ResponseObject):
    content: str | None
    function_call: JsonValue
    refusal: str | None
    role: str | None
    tool_calls: JsonValue


class StreamingChunkChoice(ResponseObject):
    delta: StreamingChunkDelta
    finish_reason: FinishReason | None
    index: int
    logprobs: JsonValue
    defaults: ClassVar[dict[str, StoredValue]] = {"index": 0}


class ChunkUsage(ResponseObject):
    completion_tokens: int
    prompt_tokens: int
    total_tokens: int
    completion_tokens_details: JsonValue
    prompt_tokens_details: JsonValue


class StreamingChatCompletionChunk(ResponseObject):
    id: str
    choices: list[StreamingChunkChoice]
    created: int
    model: str
    object: str
    moderation: JsonValue
    service_tier: str | None
    system_fingerprint: str | None
    usage: ChunkUsage | None

    @override
    def __init__(self, **kwargs: StoredValue) -> None:
        converted: list[StoredValue] = []
        match kwargs["choices"]:
            case Sequence():
                for choice in kwargs["choices"]:
                    match choice:
                        case Mapping():
                            converted.append(chunk_choice(choice))
                        case _other:
                            converted.append(choice)
            case _other:
                raise ValidationError(
                    "StreamingChatCompletionChunk", ("choices",), "wrong type"
                )
        kwargs["choices"] = converted
        super().__init__(**kwargs)


def chunk_choice(choice: Mapping[str, StoredValue]) -> StoredValue:
    dumped = StreamingChoices(**choice).model_dump()
    match dumped:
        case Mapping():
            return StreamingChunkChoice(**dumped)
        case _other:
            return choice


def response_usage(value: StoredValue) -> StoredValue:
    match value:
        case Usage():
            return value
        case Mapping():
            return Usage(**value)
        case ResponseObject():
            dumped = value.model_dump()
            match dumped:
                case Mapping():
                    return Usage(**dumped)
                case _other:
                    return value
        case _other:
            return value


__all__ = [
    "OPENAI_RESPONSE_HEADERS", "ModelResponse", "ModelResponseBase", "ModelResponseStream",
    "StreamingChatCompletionChunk", "_generate_id", "process_response_headers",
]
