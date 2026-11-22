import asyncio
import json
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path
from typing import Final, NoReturn

import litellm
import pytest
from completion_server import (
    GOOD_KEY,
    GOOD_MODEL,
    LITELLM,
    LITELLM_ASYNC,
    LITELLM_ASYNC_CLASSES,
    LITELLM_CHAT,
    LITELLM_CLASSES,
    LITELLM_CLIENT,
    UNKNOWN_MODEL,
    AsyncChatView,
    AsyncCreate,
    CompletionServer,
    RefResult,
    dump_of,
    litellm_async_plain,
    litellm_plain,
    litellm_stream_parts,
    model_response,
    normalized,
    ours_stream_parts,
    stream_response,
)
from pydantic import JsonValue

from betterlite._completion import adapters, arguments
from betterlite._completion.arguments import JSON, CallValue
from betterlite._completion.retries import (
    acompletion_with_retries,
    attempts_for,
    backoff_seconds,
    completion_with_retries,
    retry_kwargs,
)
from betterlite._completion.stream import CompletionOutcome
from betterlite.main import (
    AsyncCompletions,
    Chat,
    Completions,
    CustomStreamWrapper,
    LiteLLM,
    aadapter_completion,
    adapter_completion,
    config_completion,
)
from betterlite.types.utils import Message, ModelResponse

MESSAGES: Final[list[dict[str, JsonValue] | Message]] = [{"role": "user", "content": "hi"}]
PLAIN_MESSAGES: Final[list[dict[str, JsonValue]]] = [{"role": "user", "content": "hi"}]
TEMPERATURE: Final = 0.5
MAX_TOKENS: Final = 7
CLIENT_TIMEOUT: Final = 600
ROUTED_ID: Final = "chatcmpl-routed"
ROUTED_CREATED: Final = 1700000000
ROUTED_CONTENT: Final = "routed"
FAILURE_TEXT: Final = "attempt failed"
SYNC_RETRIES: Final = 2
ASYNC_RETRIES: Final = 2
DEFAULT_ATTEMPTS: Final = 3
CONFIGURED_RETRIES: Final = 5
SECOND_RETRIES: Final = 7
MIN_ATTEMPTS: Final = 1
FIRST_BACKOFF: Final = 1.0
FOUR_SECONDS: Final = 4.0
CAP_ATTEMPT: Final = 5
BACKOFF_CAP: Final = 10.0
CONFIG_NAME: Final = "config.json"
CONFIG_MESSAGE: Final = (
    "No config path set, please set a config path using "
    "`litellm.config_path = 'path/to/config.json'`"
)
ADAPTER_ID: Final = "fake-adapter"
ADAPTER_CALLS: Final = 1
ZERO_CALLS: Final = 0


def routed_response() -> ModelResponse:
    return ModelResponse(
        id=ROUTED_ID, created=ROUTED_CREATED, model=GOOD_MODEL,
        choices=[{"index": 0, "message": {"role": "assistant", "content": ROUTED_CONTENT}}],
    )


class RecordingRouter:
    def __init__(self) -> None:
        self.calls: list[
            tuple[str | None, list[dict[str, JsonValue] | Message], dict[str, JsonValue]]
        ] = []

    def completion(
        self,
        model: str | None,
        messages: list[dict[str, JsonValue] | Message],
        **params: JsonValue,
    ) -> CompletionOutcome:
        self.calls.append((model, messages, params))
        return routed_response()

    async def acompletion(
        self,
        model: str | None,
        messages: list[dict[str, JsonValue] | Message],
        **params: JsonValue,
    ) -> CompletionOutcome:
        await asyncio.sleep(0)
        return self.completion(model, messages, **params)


class Counter:
    def __init__(self) -> None:
        self.calls: list[dict[str, CallValue]] = []

    def failing(self, *_args: JsonValue, **kwargs: CallValue) -> NoReturn:
        self.calls.append(dict(kwargs))
        raise ValueError(FAILURE_TEXT)


class Keeper:
    def __init__(self) -> None:
        self.response = routed_response()
        self.calls = 0

    def keep(self, *_args: JsonValue, **_kwargs: CallValue) -> CompletionOutcome:
        self.calls += 1
        return self.response


class FakeAdapter:
    def __init__(self) -> None:
        self.input_calls = 0
        self.output_calls = 0
        self.stream_calls = 0

    def translate_completion_input_params(  # ast-grep-ignore: no-long-names
        self, *, kwargs: dict[str, JsonValue]
    ) -> dict[str, JsonValue]:
        self.input_calls += 1
        return {**kwargs, "model": GOOD_MODEL}

    def translate_completion_output_params(  # ast-grep-ignore: no-long-names
        self, *, response: ModelResponse
    ) -> ModelResponse:
        self.output_calls += 1
        return response

    def translate_completion_output_params_streaming(  # ast-grep-ignore: no-long-names
        self, *, completion_stream: CustomStreamWrapper
    ) -> CompletionOutcome | None:
        self.stream_calls += 1
        return completion_stream


def retries_params(retries: int) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    arguments.num_retries = retries
    litellm.num_retries = retries
    try:
        ours = LiteLLM(api_key=GOOD_KEY)
        reference = LITELLM_CLIENT.LiteLLM(api_key=GOOD_KEY)
    finally:
        arguments.num_retries = None
        litellm.num_retries = None
    return ours.params, reference.params


def async_chat(server_url: str) -> tuple[AsyncCompletions, AsyncChatView]:
    params: dict[str, JsonValue] = {
        "api_key": GOOD_KEY, "base_url": server_url, "acompletion": True,
    }
    reference = LITELLM_CHAT.Chat(dict(params), None)
    chat = Chat(params, None)
    assert "acompletion" not in params
    assert reference.params == params
    match chat.completions:
        case AsyncCompletions() as completions:
            return completions, reference
        case Completions():
            raise AssertionError("a client with acompletion picked the sync completions")


@contextmanager
def adapter_seam(adapter: FakeAdapter) -> Iterator[None]:
    adapters.adapter_entries = [{"id": ADAPTER_ID, "adapter": adapter}]
    try:
        yield
    finally:
        adapters.adapter_entries = None


@contextmanager
def config_seam(config_file: Path) -> Iterator[None]:
    arguments.config_path = str(config_file)
    try:
        yield
    finally:
        arguments.config_path = None


def adapter_outcome(
    adapter: FakeAdapter, server_url: str, *, stream: bool = False
) -> CompletionOutcome | None:
    with adapter_seam(adapter):
        return adapter_completion(
            adapter_id=ADAPTER_ID, model=UNKNOWN_MODEL, messages=MESSAGES,
            api_base=server_url, api_key=GOOD_KEY, stream=stream,
        )


def config_call(config_file: Path) -> dict[str, JsonValue]:
    with config_seam(config_file):
        return dump_of(config_completion(model=UNKNOWN_MODEL, messages=MESSAGES))


def plain_reference(server_url: str) -> dict[str, JsonValue]:
    return litellm_plain(
        model=GOOD_MODEL, messages=PLAIN_MESSAGES, api_base=server_url, api_key=GOOD_KEY
    )


def write_config(server_url: str, config_file: Path) -> None:
    config: dict[str, JsonValue] = {
        "model": GOOD_MODEL, "messages": JSON.validate_python(PLAIN_MESSAGES),
        "api_base": server_url, "api_key": GOOD_KEY,
    }
    config_file.write_text(json.dumps(config))


def test_client_plain_matches() -> None:
    with CompletionServer() as server:
        client = LiteLLM(api_key=GOOD_KEY, base_url=server.url)
        reference_client = LITELLM_CLIENT.LiteLLM(api_key=GOOD_KEY, base_url=server.url)
        response = model_response(
            client.chat.completions.create(messages=MESSAGES, model=GOOD_MODEL)
        )
        reference = litellm_plain(
            model=GOOD_MODEL, messages=PLAIN_MESSAGES, api_base=server.url, api_key=GOOD_KEY
        )
        expected_params = {
            name: value for name, value in reference_client.params.items() if name != "self"
        }
    assert normalized(response.model_dump()) == normalized(reference)
    assert client.params == expected_params
    assert client.params["timeout"] == CLIENT_TIMEOUT
    assert client.params["max_retries"] is None
    assert "self" not in client.params


def test_client_reads_retries() -> None:
    first, reference_first = retries_params(CONFIGURED_RETRIES)
    second, _reference_second = retries_params(SECOND_RETRIES)
    assert first["max_retries"] == CONFIGURED_RETRIES
    assert second["max_retries"] == SECOND_RETRIES
    assert reference_first["max_retries"] is None


async def async_chat_response(completions: AsyncCreate) -> dict[str, JsonValue]:
    response = await completions.create(messages=MESSAGES, model=GOOD_MODEL)
    return dump_of(response)


async def chat_responses(
    completions: AsyncCompletions, reference: AsyncChatView
) -> tuple[dict[str, JsonValue], dict[str, JsonValue]]:
    ours = await completions.create(messages=MESSAGES, model=GOOD_MODEL)
    reference_response = await reference.completions.create(
        messages=MESSAGES, model=GOOD_MODEL
    )
    return dump_of(ours), dump_of(reference_response)


def test_client_async_branch() -> None:
    with CompletionServer() as server:
        completions, reference = async_chat(server.url)
        ours, reference_response = asyncio.run(chat_responses(completions, reference))
        expected = asyncio.run(
            litellm_async_plain(
                model=GOOD_MODEL, messages=PLAIN_MESSAGES, api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
    assert [normalized(ours), normalized(reference_response)] == [normalized(expected)] * 2


def test_client_router() -> None:
    ours_router = RecordingRouter()
    reference_router = RecordingRouter()
    ours_params: dict[str, JsonValue] = {"temperature": TEMPERATURE}
    reference_params: dict[str, JsonValue] = {"temperature": TEMPERATURE}
    ours = Completions(ours_params, ours_router)
    reference = LITELLM_CLASSES.Completions(reference_params, reference_router)
    ours_outcome = ours.create(messages=MESSAGES, model=GOOD_MODEL, max_tokens=MAX_TOKENS)
    reference_outcome = reference.create(
        messages=MESSAGES, model=GOOD_MODEL, max_tokens=MAX_TOKENS
    )
    assert dump_of(ours_outcome) == dump_of(reference_outcome)
    ours.create(messages=MESSAGES, model=None)
    reference.create(messages=MESSAGES, model=None)
    assert ours_router.calls == reference_router.calls
    assert ours_params == reference_params
    assert ours_params["max_tokens"] == MAX_TOKENS
    assert ours_router.calls[1][0] is None


async def our_async_router_call(completions: AsyncCompletions) -> dict[str, JsonValue]:
    outcome = await completions.create(messages=MESSAGES, model=GOOD_MODEL)
    return dump_of(outcome)


async def reference_async_router_call(completions: AsyncCreate) -> dict[str, JsonValue]:
    outcome = await completions.create(messages=MESSAGES, model=GOOD_MODEL)
    return dump_of(outcome)


def test_client_async_router() -> None:
    ours_router = RecordingRouter()
    reference_router = RecordingRouter()
    ours = AsyncCompletions({"temperature": TEMPERATURE}, ours_router)
    reference = LITELLM_ASYNC_CLASSES.AsyncCompletions(
        {"temperature": TEMPERATURE}, reference_router
    )
    ours_outcome = asyncio.run(our_async_router_call(ours))
    reference_outcome = asyncio.run(reference_async_router_call(reference))
    assert ours_outcome == reference_outcome
    assert ours_router.calls == reference_router.calls


def test_retries_sync_matches() -> None:
    ours_counter = Counter()
    with pytest.raises(ValueError, match=FAILURE_TEXT) as ours_caught:
        completion_with_retries(
            model=GOOD_MODEL, messages=MESSAGES, num_retries=SYNC_RETRIES,
            original_function=ours_counter.failing,
        )
    reference_counter = Counter()
    with pytest.raises(ValueError, match=FAILURE_TEXT) as reference_caught:
        LITELLM.completion_with_retries(
            model=GOOD_MODEL, messages=PLAIN_MESSAGES, num_retries=SYNC_RETRIES,
            original_function=reference_counter.failing,
        )
    assert len(ours_counter.calls) == SYNC_RETRIES
    assert ours_counter.calls == reference_counter.calls
    assert str(ours_caught.value) == str(reference_caught.value)


async def ours_async_retries(counter: Counter) -> CompletionOutcome:
    return await acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=ASYNC_RETRIES,
        original_function=counter.failing,
    )


async def reference_async_retries(counter: Counter) -> RefResult:
    return await LITELLM_ASYNC.acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=ASYNC_RETRIES,
        original_function=counter.failing,
    )


def test_retries_async_matches() -> None:
    ours_counter = Counter()
    reference_counter = Counter()
    with pytest.raises(ValueError, match=FAILURE_TEXT) as ours_caught:
        asyncio.run(ours_async_retries(ours_counter))
    with pytest.raises(ValueError, match=FAILURE_TEXT) as reference_caught:
        asyncio.run(reference_async_retries(reference_counter))
    assert ours_counter.calls == reference_counter.calls
    assert len(ours_counter.calls) == ASYNC_RETRIES
    assert str(ours_caught.value) == str(reference_caught.value)


async def ours_keep_outcome(keeper: Keeper) -> CompletionOutcome:
    return await acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=MIN_ATTEMPTS,
        original_function=keeper.keep,
    )


async def reference_keep_outcome(keeper: Keeper) -> RefResult:
    return await LITELLM_ASYNC.acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=MIN_ATTEMPTS,
        original_function=keeper.keep,
    )


def test_retries_keep_sync_outcome() -> None:
    ours = Keeper()
    assert asyncio.run(ours_keep_outcome(ours)) is ours.response
    reference = Keeper()
    assert asyncio.run(reference_keep_outcome(reference)) is reference.response
    assert ours.calls == reference.calls


def test_retries_helpers() -> None:
    assert attempts_for(None) == DEFAULT_ATTEMPTS
    assert attempts_for(0) == MIN_ATTEMPTS
    assert attempts_for(SYNC_RETRIES) == SYNC_RETRIES
    assert backoff_seconds(MIN_ATTEMPTS) == FIRST_BACKOFF
    assert backoff_seconds(CAP_ATTEMPT - 2) == FOUR_SECONDS
    assert backoff_seconds(CAP_ATTEMPT) == BACKOFF_CAP
    constant: dict[str, CallValue] = {}
    assert not retry_kwargs(constant, "constant_retry")
    assert constant == {"num_retries": 0, "max_retries": 0}
    assert retry_kwargs({}, "exponential_backoff_retry")
    assert not retry_kwargs({}, None)


def test_config_matches(tmp_path: Path) -> None:
    config_file = tmp_path / CONFIG_NAME
    with CompletionServer() as server:
        write_config(server.url, config_file)
        ours = config_call(config_file)
        reference = plain_reference(server.url)
    assert normalized(ours) == normalized(reference)


def test_config_without_path() -> None:
    with pytest.raises(ValueError, match="No config path set") as ours_caught:
        config_completion(model=GOOD_MODEL, messages=MESSAGES)
    with pytest.raises(ValueError, match="No config path set") as reference_caught:
        LITELLM.config_completion()
    assert str(ours_caught.value) == str(reference_caught.value) == CONFIG_MESSAGE


def test_adapter_translates() -> None:
    adapter = FakeAdapter()
    with CompletionServer() as server:
        ours = dump_of(adapter_outcome(adapter, server.url))
        reference = plain_reference(server.url)
    assert normalized(ours) == normalized(reference)
    assert adapter.input_calls == ADAPTER_CALLS
    assert adapter.output_calls == ADAPTER_CALLS
    assert adapter.stream_calls == ZERO_CALLS


def test_adapter_stream_matches() -> None:
    adapter = FakeAdapter()
    with CompletionServer() as server:
        outcome = adapter_outcome(adapter, server.url, stream=True)
        ours = ours_stream_parts(stream_response(outcome))
        reference = litellm_stream_parts(
            model=GOOD_MODEL, messages=PLAIN_MESSAGES, api_base=server.url,
            api_key=GOOD_KEY, stream=True,
        )
    assert ours == reference
    assert adapter.stream_calls == ADAPTER_CALLS
    assert adapter.output_calls == ZERO_CALLS


async def ours_adapt_async(adapter: FakeAdapter, server_url: str) -> dict[str, JsonValue]:
    with adapter_seam(adapter):
        outcome = await aadapter_completion(
            adapter_id=ADAPTER_ID, model=UNKNOWN_MODEL, messages=MESSAGES,
            api_base=server_url, api_key=GOOD_KEY,
        )
        return dump_of(outcome)


async def reference_adapt_async(server_url: str) -> dict[str, JsonValue]:
    return await litellm_async_plain(
        model=GOOD_MODEL, messages=PLAIN_MESSAGES, api_base=server_url, api_key=GOOD_KEY
    )


def test_adapter_async_matches() -> None:
    adapter = FakeAdapter()
    with CompletionServer() as server:
        ours = asyncio.run(ours_adapt_async(adapter, server.url))
        reference = asyncio.run(reference_adapt_async(server.url))
    assert normalized(ours) == normalized(reference)
    assert [adapter.input_calls, adapter.output_calls] == [ADAPTER_CALLS, ADAPTER_CALLS]


def test_adapter_missing() -> None:
    with pytest.raises(ValueError, match="No matching adapter given") as ours_caught:
        adapter_completion(adapter_id=ADAPTER_ID, model=GOOD_MODEL, messages=MESSAGES)
    with pytest.raises(ValueError, match="No matching adapter given") as reference_caught:
        LITELLM.adapter_completion(
            adapter_id=ADAPTER_ID, model=GOOD_MODEL, messages=PLAIN_MESSAGES
        )
    assert str(ours_caught.value) == str(reference_caught.value)
