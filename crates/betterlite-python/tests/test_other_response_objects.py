import asyncio
import json
import logging
from typing import Protocol

import litellm.types.utils as litellm_utils  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1
import pytest
from openai.types.image import Image as OpenAIImage
from typing_extensions import override

import betterlite.types.utils as betterlite_utils
from betterlite.types.utils.adapter_stream import (
    AdapterCompletionStreamWrapper,
    AdapterStreamError,
    CustomStreamingDecoder,
)
from betterlite.types.utils.media import Selection
from betterlite.types.utils.values import JsonValue, StoredValue

INDEX = 3
CHOICE_INDEX = 2
CREATED = 5
RESPONSE_MS = 5.0
PROMPT_TOKENS = 1
COMPLETION_TOKENS = 2
TOTAL_TOKENS = 3
EPOCHS = 3
SEED = 4
BUDGET_MAX = 5.5
BUDGET_TPM = 10
BUDGET_LIMIT = 7
OTHER_BUDGET = 9
DURATION_SECONDS = 1.5
IMAGE_COUNT = 2
AUDIO_TOKENS = 1
TEXT_TOKENS = 2


class Dumpable(Protocol):
    def model_dump(
        self, *, exclude_unset: bool = False, exclude_none: bool = False,
    ) -> JsonValue: ...


class RecordingHandler(logging.Handler):
    messages: list[str]

    def __init__(self) -> None:
        super().__init__()
        self.messages = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


class ChunkStream:
    chunks: list[StoredValue]

    def __init__(self, chunks: list[StoredValue]) -> None:
        self.chunks = chunks

    def __iter__(self) -> "ChunkStream":
        return self

    def __next__(self) -> StoredValue:
        if not self.chunks:
            raise StopIteration
        return self.chunks.pop(0)


class AsyncChunkStream:
    chunks: list[StoredValue]

    def __init__(self, chunks: list[StoredValue]) -> None:
        self.chunks = chunks

    def __aiter__(self) -> "AsyncChunkStream":
        return self

    async def __anext__(self) -> StoredValue:
        if not self.chunks:
            raise StopAsyncIteration
        return self.chunks.pop(0)


class ByteStream:
    chunks: list[bytes]

    def __init__(self, chunks: list[bytes]) -> None:
        self.chunks = chunks

    def __aiter__(self) -> "ByteStream":
        return self

    async def __anext__(self) -> bytes:
        if not self.chunks:
            raise StopAsyncIteration
        return self.chunks.pop(0)


def parse_object(text: str) -> dict[str, JsonValue]:
    parsed: dict[str, JsonValue] = json.loads(text)
    return parsed


def assert_same(label: str, reference: Dumpable, ours: Dumpable) -> None:
    assert ours.model_dump() == reference.model_dump(), label
    assert ours.model_dump(exclude_unset=True) == reference.model_dump(exclude_unset=True), label
    assert ours.model_dump(exclude_none=True) == reference.model_dump(exclude_none=True), label


def test_embedding_defaults() -> None:
    reference = litellm_utils.Embedding(index=0, object="embedding")
    ours = betterlite_utils.Embedding(index=0, object="embedding")
    assert ours.embedding == []
    assert ours.index == 0
    assert ours.object == "embedding"
    assert_same("embedding defaults", reference, ours)


def test_embedding_values() -> None:
    embedding_values: list[JsonValue] = [1.0, 2.5]
    reference = litellm_utils.Embedding(embedding=embedding_values, index=INDEX, object="embedding")
    ours = betterlite_utils.Embedding(embedding=embedding_values, index=INDEX, object="embedding")
    reference["extra"] = "e"
    ours["extra"] = "e"
    assert ours.embedding == embedding_values
    assert ours["extra"] == "e"
    assert ours.get("index") == INDEX
    assert_same("embedding values", reference, ours)
    text_reference = litellm_utils.Embedding(embedding="text", index=0, object="embedding")
    text_ours = betterlite_utils.Embedding(embedding="text", index=0, object="embedding")
    assert text_ours.embedding == "text"
    assert_same("embedding text", text_reference, text_ours)


def test_embedding_response() -> None:
    reference = litellm_utils.EmbeddingResponse()
    ours = betterlite_utils.EmbeddingResponse()
    assert ours.object == "list"
    assert ours.model is None
    assert ours.data == []
    assert ours.usage == betterlite_utils.Usage()
    assert ours.get("data") == []
    assert "model" in ours
    assert_same("embedding response empty", reference, ours)


def test_embedding_response_data() -> None:
    embedding_values: list[JsonValue] = [1.0]
    reference_data = [litellm_utils.Embedding(
        embedding=embedding_values, index=0, object="embedding",
    )]
    ours_data = [betterlite_utils.Embedding(
        embedding=embedding_values, index=0, object="embedding",
    )]
    reference = litellm_utils.EmbeddingResponse(
        model="m", data=reference_data,
        usage=litellm_utils.Usage(prompt_tokens=PROMPT_TOKENS, total_tokens=PROMPT_TOKENS),
    )
    ours = betterlite_utils.EmbeddingResponse(
        model="m", data=ours_data,
        usage=betterlite_utils.Usage(prompt_tokens=PROMPT_TOKENS, total_tokens=PROMPT_TOKENS),
    )
    assert ours.model == "m"
    assert len(ours.data) == 1
    assert ours.usage is not None
    assert ours.usage.prompt_tokens == PROMPT_TOKENS
    assert_same("embedding response data", reference, ours)
    hidden = betterlite_utils.EmbeddingResponse(model="m", hidden_params={"a": 1})
    assert hidden._hidden_params == {"a": 1}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2298-L2300
    tuple_ours = betterlite_utils.EmbeddingResponse(data=(1, 2))
    assert tuple_ours.data == [1, 2]
    private = betterlite_utils.EmbeddingResponse(
        response_ms=RESPONSE_MS, _response_headers={"h": "v"},
    )
    assert private._response_ms == RESPONSE_MS  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2303-L2304
    assert private._response_headers == {"h": "v"}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2303-L2304
    dropped_reference = litellm_utils.EmbeddingResponse(unknown="x")
    dropped_ours = betterlite_utils.EmbeddingResponse(unknown="x")
    assert_same("embedding response dropped", dropped_reference, dropped_ours)


def test_text_completion_logprobs() -> None:
    reference = litellm_utils.Logprobs(
        text_offset=[0], token_logprobs=[-0.5], tokens=["a"], top_logprobs=[{"a": -0.5}],
    )
    ours = betterlite_utils.Logprobs(
        text_offset=[0], token_logprobs=[-0.5], tokens=["a"], top_logprobs=[{"a": -0.5}],
    )
    assert ours.tokens == ["a"]
    assert_same("logprobs", reference, ours)
    with pytest.raises(ValueError, match="validation error"):
        _ = betterlite_utils.Logprobs()


def test_text_completion_choices() -> None:
    logprobs: dict[str, JsonValue] = {
        "text_offset": [0],
        "token_logprobs": [-0.5],
        "tokens": ["a"],
        "top_logprobs": [None],
    }
    reference = litellm_utils.TextChoices(
        finish_reason="length", index=2, text="hi", logprobs=logprobs,
    )
    ours = betterlite_utils.TextChoices(
        finish_reason="length", index=2, text="hi", logprobs=logprobs,
    )
    assert ours.finish_reason == "length"
    assert ours.index == CHOICE_INDEX
    assert ours.text == "hi"
    stored = ours.logprobs
    assert stored is not None
    assert stored.tokens == ["a"]
    assert_same("text choices", reference, ours)
    empty_reference = litellm_utils.TextChoices()
    empty_ours = betterlite_utils.TextChoices()
    assert empty_ours.finish_reason is None
    assert empty_ours.index == 0
    assert empty_ours.text is None
    assert_same("text choices empty", empty_reference, empty_ours)
    mapped_reference = litellm_utils.TextChoices(finish_reason="tool_use")
    mapped_ours = betterlite_utils.TextChoices(finish_reason="tool_use")
    assert mapped_ours.finish_reason == "tool_calls"
    assert_same("text choices mapped", mapped_reference, mapped_ours)
    extra_reference = litellm_utils.TextChoices(finish_reason="stop", extra="e")
    extra_ours = betterlite_utils.TextChoices(finish_reason="stop", extra="e")
    assert extra_ours["extra"] == "e"
    assert_same("text choices extra", extra_reference, extra_ours)


def test_text_completion_response() -> None:
    choices: list[JsonValue] = [{"text": "hi", "index": 0, "finish_reason": "stop"}]
    usage: dict[str, JsonValue] = {
        "prompt_tokens": PROMPT_TOKENS, "total_tokens": TOTAL_TOKENS,
    }
    reference = litellm_utils.TextCompletionResponse(
        id="i", choices=choices, created=CREATED, model="m", extra="e", usage=usage,
    )
    ours = betterlite_utils.TextCompletionResponse(
        id="i", choices=choices, created=CREATED, model="m", extra="e", usage=usage,
    )
    assert ours.id == "i"
    assert ours.object == "text_completion"
    assert ours.created == CREATED
    assert ours.model == "m"
    assert ours.usage is not None
    assert ours.usage.total_tokens == TOTAL_TOKENS
    assert ours.choices[0].finish_reason == "stop"
    assert ours.choices[0].text == "hi"
    assert ours["extra"] == "e"
    assert_same("text completion", reference, ours)


def test_text_completion_defaults() -> None:
    ours = betterlite_utils.TextCompletionResponse()
    assert ours.id.startswith("chatcmpl-")
    assert ours.created > 0
    assert ours.model is None
    assert len(ours.choices) == 1
    assert ours.choices[0].finish_reason is None
    assert ours.usage == betterlite_utils.Usage()
    dumped = parse_object(ours.model_dump_json())
    assert dumped["object"] == "text_completion"
    assert dumped["id"] == ours.id


def test_text_completion_stream() -> None:
    reference = litellm_utils.TextCompletionResponse(
        id="i", created=CREATED, model="m", stream=True,
    )
    ours = betterlite_utils.TextCompletionResponse(id="i", created=CREATED, model="m", stream=True)
    assert ours.object == "text_completion.chunk"
    assert len(ours.choices) == 1
    assert ours.choices[0].finish_reason is None
    assert_same("text completion stream", reference, ours)
    chosen_choices: list[JsonValue] = [{"text": "x"}]
    chosen_reference = litellm_utils.TextCompletionResponse(
        id="i", created=CREATED, choices=chosen_choices, object="ignored",
    )
    chosen_ours = betterlite_utils.TextCompletionResponse(
        id="i", created=CREATED, choices=chosen_choices, object="ignored",
    )
    assert chosen_ours.object == "text_completion"
    assert chosen_ours.choices[0].text == "x"
    assert_same("text completion object", chosen_reference, chosen_ours)
    fallback_reference = litellm_utils.TextCompletionResponse(
        id="i", created=CREATED, choices="x",
    )
    fallback_ours = betterlite_utils.TextCompletionResponse(id="i", created=CREATED, choices="x")
    assert len(fallback_ours.choices) == 1
    assert_same("text completion fallback", fallback_reference, fallback_ours)
    timed_reference = litellm_utils.TextCompletionResponse(
        id="i", created=CREATED, response_ms=CREATED,
    )
    timed_ours = betterlite_utils.TextCompletionResponse(
        id="i", created=CREATED, response_ms=CREATED,
    )
    assert timed_ours._response_ms == CREATED  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2510-L2513
    assert timed_reference._response_ms == CREATED  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2510-L2513
    assert_same("text completion timed", timed_reference, timed_ours)


def test_image_objects() -> None:
    reference = litellm_utils.ImageObject()
    ours = betterlite_utils.ImageObject()
    assert ours.b64_json is None
    assert ours.url is None
    assert ours.provider_specific_fields is None
    assert_same("image object empty", reference, ours)
    provider_fields: dict[str, JsonValue] = {"p": 1}
    values_reference = litellm_utils.ImageObject(
        b64_json="b", url="u", revised_prompt="r", provider_specific_fields=provider_fields,
    )
    values_ours = betterlite_utils.ImageObject(
        b64_json="b", url="u", revised_prompt="r", provider_specific_fields=provider_fields,
    )
    assert values_ours["url"] == "u"
    assert values_ours.revised_prompt == "r"
    assert_same("image object values", values_reference, values_ours)
    url_reference = litellm_utils.ImageObject(url="u")
    url_ours = betterlite_utils.ImageObject(url="u")
    assert url_ours.get("url") == "u"
    assert_same("image object url", url_reference, url_ours)


def test_image_usage() -> None:
    details_reference = litellm_utils.ImageUsageInputTokensDetails(image_tokens=1, text_tokens=2)
    details_ours = betterlite_utils.ImageUsageInputTokensDetails(image_tokens=1, text_tokens=2)
    assert details_ours.image_tokens == 1
    assert_same("image details", details_reference, details_ours)
    reference = litellm_utils.ImageUsage(
        input_tokens=PROMPT_TOKENS, input_tokens_details=details_reference,
        output_tokens=COMPLETION_TOKENS, total_tokens=TOTAL_TOKENS,
    )
    ours = betterlite_utils.ImageUsage(
        input_tokens=PROMPT_TOKENS, input_tokens_details=details_ours,
        output_tokens=COMPLETION_TOKENS, total_tokens=TOTAL_TOKENS,
    )
    assert ours.total_tokens == TOTAL_TOKENS
    assert_same("image usage", reference, ours)


def image_usage_pair() -> tuple[litellm_utils.ImageUsage, betterlite_utils.ImageUsage]:
    reference = litellm_utils.ImageUsage(
        input_tokens=PROMPT_TOKENS,
        input_tokens_details=litellm_utils.ImageUsageInputTokensDetails(
            image_tokens=1, text_tokens=2,
        ),
        output_tokens=COMPLETION_TOKENS,
        total_tokens=TOTAL_TOKENS,
    )
    ours = betterlite_utils.ImageUsage(
        input_tokens=PROMPT_TOKENS,
        input_tokens_details=betterlite_utils.ImageUsageInputTokensDetails(
            image_tokens=1, text_tokens=2,
        ),
        output_tokens=COMPLETION_TOKENS,
        total_tokens=TOTAL_TOKENS,
    )
    return reference, ours


def image_object_pair() -> tuple[
    list[litellm_utils.ImageObject], list[betterlite_utils.ImageObject],
]:
    reference = [
        litellm_utils.ImageObject(b64_json="b0", revised_prompt="r0"),
        litellm_utils.ImageObject(url="u1"),
    ]
    ours = [
        betterlite_utils.ImageObject(b64_json="b0", revised_prompt="r0"),
        betterlite_utils.ImageObject(url="u1"),
    ]
    return reference, ours


def test_image_response() -> None:
    reference_images, ours_images = image_object_pair()
    reference_usage, ours_usage = image_usage_pair()
    hidden_params: dict[str, StoredValue] = {"a": 1}
    reference = litellm_utils.ImageResponse(
        created=CREATED, data=reference_images, usage=reference_usage,
        hidden_params=hidden_params, size="1024x1024",
    )
    ours = betterlite_utils.ImageResponse(
        created=CREATED, data=ours_images, usage=ours_usage,
        hidden_params=hidden_params, size="1024x1024",
    )
    assert ours.created == CREATED
    assert ours.size == "1024x1024"
    assert ours.background is None
    assert len(ours.data) == IMAGE_COUNT
    assert ours.usage is not None
    assert ours.usage.total_tokens == TOTAL_TOKENS
    assert ours._hidden_params == hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2623
    assert_same("image response", reference, ours)
    empty_reference = litellm_utils.ImageResponse(created=CREATED)
    empty_ours = betterlite_utils.ImageResponse(created=CREATED)
    assert empty_ours.data == []
    assert empty_ours.usage is not None
    assert empty_ours.usage.input_tokens == 0
    assert_same("image response empty", empty_reference, empty_ours)
    generated = betterlite_utils.ImageResponse()
    assert generated.created > 0
    dict_ours = betterlite_utils.ImageResponse(created=CREATED, data=[{"url": "u3"}])
    dict_reference = litellm_utils.ImageResponse(
        created=CREATED, data=[litellm_utils.ImageObject(url="u3")],
    )
    assert dict_ours.data[0].url == "u3"
    assert_same("image dict data", dict_reference, dict_ours)


def test_image_model_data() -> None:
    model = OpenAIImage(url="u2")
    reference = litellm_utils.ImageResponse(
        created=CREATED,
        data=[model],  # type: ignore[list-item]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2676-L2682
    )
    ours = betterlite_utils.ImageResponse(created=CREATED, data=[model])
    assert ours.data[0].url == "u2"
    assert_same("image model data", reference, ours)


def test_image_selection() -> None:
    reference_images, ours_images = image_object_pair()
    reference = litellm_utils.ImageResponse(created=CREATED, data=reference_images)
    ours = betterlite_utils.ImageResponse(created=CREATED, data=ours_images)
    includes: list[Selection] = [
        {"data"},
        {"data": {0}},
        {"data": {1}},
        {"data": [0]},
        {"data": {-1}},
        {"data": {0: {"url"}}},
        {"data": {"__all__": {"url"}}},
        {"data": {"__all__": {"url": False}}},
        {"data": {}},
        {"data": []},
        {"created"},
        {"created": False},
    ]
    for selector in includes:
        expected = parse_object(reference.model_dump_json(include=selector))  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2625-L2660
        assert parse_object(ours.model_dump_json(include=selector)) == expected, selector
    excludes: list[Selection] = [
        {"data"},
        {"data": {1}},
        {"data": [1]},
        {"data": {"__all__": {"url"}}},
        {"data": {"__all__": {"url": True}}},
        {"data": {"__all__": {"url": False}}},
        {"data": {}},
        {"created"},
        {"created": False},
    ]
    for selector in excludes:
        expected = parse_object(reference.model_dump_json(exclude=selector))  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2625-L2660
        assert parse_object(ours.model_dump_json(exclude=selector)) == expected, selector


def test_transcription_response() -> None:
    reference = litellm_utils.TranscriptionResponse(text="hi")
    ours = betterlite_utils.TranscriptionResponse(text="hi")
    assert ours.text == "hi"
    assert ours.usage is None
    assert_same("transcription", reference, ours)
    duration_reference = litellm_utils.TranscriptionUsageDurationObject(
        type="duration", seconds=DURATION_SECONDS,
    )
    duration_ours = betterlite_utils.TranscriptionUsageDurationObject(
        type="duration", seconds=DURATION_SECONDS,
    )
    assert duration_ours.seconds == DURATION_SECONDS
    assert_same("transcription duration", duration_reference, duration_ours)
    details_reference = litellm_utils.TranscriptionUsageInputTokenDetailsObject(
        audio_tokens=AUDIO_TOKENS, text_tokens=TEXT_TOKENS,
    )
    details_ours = betterlite_utils.TranscriptionUsageInputTokenDetailsObject(
        audio_tokens=AUDIO_TOKENS, text_tokens=TEXT_TOKENS,
    )
    assert details_ours.audio_tokens == AUDIO_TOKENS
    assert_same("transcription details", details_reference, details_ours)
    tokens_reference = litellm_utils.TranscriptionUsageTokensObject(
        type="tokens", input_tokens=PROMPT_TOKENS, output_tokens=COMPLETION_TOKENS,
        total_tokens=TOTAL_TOKENS, input_token_details=details_reference,
    )
    tokens_ours = betterlite_utils.TranscriptionUsageTokensObject(
        type="tokens", input_tokens=PROMPT_TOKENS, output_tokens=COMPLETION_TOKENS,
        total_tokens=TOTAL_TOKENS, input_token_details=details_ours,
    )
    assert tokens_ours.total_tokens == TOTAL_TOKENS
    assert_same("transcription tokens", tokens_reference, tokens_ours)
    used_reference = litellm_utils.TranscriptionResponse(text="hi")
    used_reference.usage = duration_reference
    used_reference["language"] = "en"
    used_reference["duration"] = DURATION_SECONDS
    used_ours = betterlite_utils.TranscriptionResponse(text="hi")
    used_ours.usage = duration_ours
    used_ours["language"] = "en"
    used_ours["duration"] = DURATION_SECONDS
    assert used_ours["language"] == "en"
    assert used_ours.usage == duration_ours
    assert_same("transcription used", used_reference, used_ours)


def test_adapter_stream_sync() -> None:
    handler = RecordingHandler()
    logger = logging.getLogger("LiteLLM")
    previous_level = logger.level
    logger.setLevel(logging.DEBUG)
    logger.addHandler(handler)
    try:
        wrapper = AdapterCompletionStreamWrapper(ChunkStream(["a", None, "None", "b"]))
        assert iter(wrapper) is wrapper
        assert next(wrapper) == "a"
        assert next(wrapper) is None
        assert next(wrapper) is None
        assert next(wrapper) == "b"
        with pytest.raises(StopIteration):
            _ = next(wrapper)
        empty = AdapterCompletionStreamWrapper(ChunkStream([]))
        with pytest.raises(StopIteration):
            _ = next(empty)
        asynchronous = AdapterCompletionStreamWrapper(AsyncChunkStream([]))
        with pytest.raises(TypeError):
            _ = next(asynchronous)
    finally:
        logger.removeHandler(handler)
        logger.setLevel(previous_level)
    assert handler.messages == [
        "AdapterCompletionStreamWrapper - ",
        "AdapterCompletionStreamWrapper - ",
    ]


def test_adapter_stream_async() -> None:
    loop = asyncio.new_event_loop()
    try:
        loop.run_until_complete(adapter_stream_async_case())
    finally:
        loop.close()


async def adapter_stream_async_case() -> None:
    wrapper = AdapterCompletionStreamWrapper(AsyncChunkStream(["a"]))
    assert wrapper.__aiter__() is wrapper
    assert await wrapper.__anext__() == "a"
    with pytest.raises(StopAsyncIteration):
        _ = await wrapper.__anext__()
    failing = AdapterCompletionStreamWrapper(AsyncChunkStream([None]))
    with pytest.raises(AdapterStreamError):
        _ = await failing.__anext__()
    empty = AdapterCompletionStreamWrapper(AsyncChunkStream([]))
    with pytest.raises(StopAsyncIteration):
        _ = await empty.__anext__()
    synchronous = AdapterCompletionStreamWrapper(ChunkStream([]))
    with pytest.raises(TypeError):
        _ = await synchronous.__anext__()


def test_adapter_stream_decoder() -> None:
    decoder = CustomStreamingDecoder()
    with pytest.raises(NotImplementedError):
        _ = decoder.iter_bytes(iter([b"a"]))
    with pytest.raises(NotImplementedError):
        asyncio.run(decoder.aiter_bytes(ByteStream([b"a"])))


def test_small_types_budget() -> None:
    reference = litellm_utils.BudgetConfig()
    ours = betterlite_utils.BudgetConfig()
    assert ours.max_budget is None
    assert_same("budget empty", reference, ours)
    values: dict[str, StoredValue] = {
        "max_budget": BUDGET_MAX,
        "budget_duration": "1d",
        "tpm_limit": BUDGET_TPM,
        "rpm_limit": BUDGET_LIMIT,
    }
    values_reference = litellm_utils.BudgetConfig(**values)
    values_ours = betterlite_utils.BudgetConfig(**values)
    assert values_ours.max_budget == BUDGET_MAX
    assert values_ours.tpm_limit == BUDGET_TPM
    assert_same("budget values", values_reference, values_ours)
    aliases: dict[str, StoredValue] = {"time_period": "2h", "budget_limit": BUDGET_LIMIT}
    alias_reference = litellm_utils.BudgetConfig(**aliases)
    alias_ours = betterlite_utils.BudgetConfig(**aliases)
    assert alias_ours.budget_duration == "2h"
    assert alias_ours.max_budget == BUDGET_LIMIT
    assert_same("budget aliases", alias_reference, alias_ours)
    both: dict[str, StoredValue] = {
        "time_period": "2h",
        "budget_duration": "1d",
        "budget_limit": BUDGET_LIMIT,
        "max_budget": OTHER_BUDGET,
    }
    both_reference = litellm_utils.BudgetConfig(**both)
    both_ours = betterlite_utils.BudgetConfig(**both)
    assert both_ours.budget_duration == "2h"
    assert both_ours.max_budget == BUDGET_LIMIT
    assert_same("budget both", both_reference, both_ours)
    unknown_reference = litellm_utils.BudgetConfig(unknown="x")
    unknown_ours = betterlite_utils.BudgetConfig(unknown="x")
    assert_same("budget unknown", unknown_reference, unknown_ours)


def test_small_types_logging() -> None:
    base = betterlite_utils.LiteLLMLoggingBaseClass()
    base.pre_call(input=[], api_key="k")
    base.post_call(original_response={})


def test_small_types_fine_tuning() -> None:
    job: dict[str, JsonValue] = {
        "id": "job-1",
        "created_at": CREATED,
        "model": "m",
        "object": "fine_tuning.job",
        "organization_id": "org",
        "result_files": ["f"],
        "status": "succeeded",
        "training_file": "t",
        "hyperparameters": {"n_epochs": EPOCHS},
    }
    reference = litellm_utils.LiteLLMFineTuningJob(**job)
    ours = betterlite_utils.LiteLLMFineTuningJob(**job)
    assert ours.seed is None
    assert ours.hyperparameters.n_epochs == EPOCHS
    assert_same("fine tuning job", reference, ours)
    error_none: dict[str, JsonValue] = {"code": None, "message": None, "param": None}
    none_reference = litellm_utils.LiteLLMFineTuningJob(**job, error=error_none)
    none_ours = betterlite_utils.LiteLLMFineTuningJob(**job, error=error_none)
    assert none_ours.error is None
    assert_same("fine tuning error none", none_reference, none_ours)
    error: dict[str, JsonValue] = {"code": "c", "message": "m"}
    error_reference = litellm_utils.LiteLLMFineTuningJob(**job, error=error)
    error_ours = betterlite_utils.LiteLLMFineTuningJob(**job, error=error)
    kept = error_ours.error
    assert kept is not None
    assert kept.code == "c"
    assert_same("fine tuning error", error_reference, error_ours)
    seeded_reference = litellm_utils.LiteLLMFineTuningJob(**job, seed=SEED)
    seeded_ours = betterlite_utils.LiteLLMFineTuningJob(**job, seed=SEED)
    assert seeded_ours.seed == SEED
    assert_same("fine tuning seed", seeded_reference, seeded_ours)
    hidden = betterlite_utils.LiteLLMFineTuningJob(**job, _hidden_params={"a": 1})
    assert hidden._hidden_params == {"a": 1}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4307-L4317
    assert betterlite_utils.LiteLLMFineTuningJob(**job)._hidden_params == {}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4308
    empty_error: dict[str, JsonValue] = {}
    empty_reference = litellm_utils.LiteLLMFineTuningJob(**job, error=empty_error)
    empty_ours = betterlite_utils.LiteLLMFineTuningJob(**job, error=empty_error)
    assert empty_ours.error is None
    assert_same("fine tuning error empty", empty_reference, empty_ours)
    with pytest.raises(ValueError, match="validation error"):
        _ = betterlite_utils.LiteLLMFineTuningJob(id="x")


def test_small_types_realtime() -> None:
    events: list[JsonValue] = [{"type": "response.done"}, {"type": "rate_limits.updated"}]
    reference = litellm_utils.LiteLLMRealtimeStreamLoggingObject(
        results=events,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4349
        usage=litellm_utils.Usage(
            prompt_tokens=PROMPT_TOKENS, completion_tokens=COMPLETION_TOKENS,
            total_tokens=TOTAL_TOKENS,
        ),
    )
    ours = betterlite_utils.LiteLLMRealtimeStreamLoggingObject(
        results=events,
        usage=betterlite_utils.Usage(
            prompt_tokens=PROMPT_TOKENS, completion_tokens=COMPLETION_TOKENS,
            total_tokens=TOTAL_TOKENS,
        ),
    )
    assert ours.results == events
    assert ours.usage.total_tokens == TOTAL_TOKENS
    assert ours.service_tier is None
    assert_same("realtime", reference, ours)
    tier_reference = litellm_utils.LiteLLMRealtimeStreamLoggingObject(
        results=events,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4349
        usage=litellm_utils.Usage(
            prompt_tokens=PROMPT_TOKENS, completion_tokens=COMPLETION_TOKENS,
            total_tokens=TOTAL_TOKENS,
        ),
        service_tier="auto",
    )
    tier_ours = betterlite_utils.LiteLLMRealtimeStreamLoggingObject(
        results=events,
        usage=betterlite_utils.Usage(
            prompt_tokens=PROMPT_TOKENS, completion_tokens=COMPLETION_TOKENS,
            total_tokens=TOTAL_TOKENS,
        ),
        service_tier="auto",
    )
    assert tier_ours.service_tier == "auto"
    assert_same("realtime tier", tier_reference, tier_ours)
    with pytest.raises(ValueError, match="validation error"):
        _ = betterlite_utils.LiteLLMRealtimeStreamLoggingObject(results=events)


def test_small_types_prompt_params() -> None:
    ours = betterlite_utils.DynamicPromptManagementParamLiteral.list_all_params()
    assert ours == [
        "cache_control_injection_points", "knowledge_bases", "vector_store_ids",
    ]
    member = betterlite_utils.DynamicPromptManagementParamLiteral.KNOWLEDGE_BASES
    assert member.value == "knowledge_bases"
    assert member == "knowledge_bases"


def test_small_types_exports() -> None:
    names = [
        "AdapterCompletionStreamWrapper",
        "BudgetConfig",
        "CustomStreamingDecoder",
        "DynamicPromptManagementParamLiteral",
        "Embedding",
        "EmbeddingResponse",
        "ImageObject",
        "ImageResponse",
        "ImageUsage",
        "ImageUsageInputTokensDetails",
        "LiteLLMFineTuningJob",
        "LiteLLMLoggingBaseClass",
        "LiteLLMRealtimeStreamLoggingObject",
        "Logprobs",
        "TextChoices",
        "TextCompletionResponse",
        "TranscriptionResponse",
        "TranscriptionUsageDurationObject",
        "TranscriptionUsageInputTokenDetailsObject",
        "TranscriptionUsageTokensObject",
    ]
    for name in names:
        assert name in betterlite_utils.__all__
