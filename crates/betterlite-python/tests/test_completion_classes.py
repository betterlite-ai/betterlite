import asyncio
import json
from pathlib import Path
from typing import Final, NoReturn

import pytest
from completion_server import (
    GOOD_KEY,
    GOOD_MODEL,
    UNKNOWN_MODEL,
    CompletionServer,
    Dumpable,
    await_outcome,
    litellm_async_plain,
    litellm_module,
    litellm_plain,
    model_response,
    normalized,
    raised,
)
from pydantic import JsonValue

import betterlite
from betterlite._completion.arguments import CallValue
from betterlite._completion.completion import CompletionOutcome
from betterlite._completion.retries import attempts_for, backoff_seconds, retry_kwargs
from betterlite._completion.stream import CustomStreamWrapper
from betterlite.main import (
    AsyncCompletions,
    Chat,
    Completions,
    LiteLLM,
    aadapter_completion,
    acompletion_with_retries,
    adapter_completion,
    completion_with_retries,
    config_completion,
)
from betterlite.types.utils import Message, ModelResponse

MESSAGES: Final[list[dict[str, JsonValue]]] = [{"role": "user", "content": "hi"}]
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


def routed_response() -> ModelResponse:
    return ModelResponse(
        id=ROUTED_ID, created=ROUTED_CREATED, model=GOOD_MODEL,
        choices=[{"index": 0, "message": {"role": "assistant", "content": ROUTED_CONTENT}}],
    )


def dump_of(result: object) -> dict[str, JsonValue]:
    assert isinstance(result, Dumpable)
    return result.model_dump()


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
        self.calls.append((model, messages, params))
        return routed_response()


class Counter:
    def __init__(self) -> None:
        self.calls: list[dict[str, object]] = []

    def failing(self, *_args: object, **kwargs: object) -> NoReturn:
        self.calls.append(dict(kwargs))
        raise ValueError(FAILURE_TEXT)


class Keeper:
    def __init__(self) -> None:
        self.response = routed_response()

    def keep(self, **_kwargs: object) -> CompletionOutcome:
        return self.response


class FakeAdapter:
    def __init__(self) -> None:
        self.input_calls = 0
        self.output_calls = 0

    def translate_completion_input_params(
        self, *, kwargs: dict[str, JsonValue]
    ) -> dict[str, JsonValue]:
        self.input_calls += 1
        return {**kwargs, "model": GOOD_MODEL}

    def translate_completion_output_params(self, *, response: ModelResponse) -> ModelResponse:
        self.output_calls += 1
        return response

    def translate_completion_output_params_streaming(
        self, *, completion_stream: CustomStreamWrapper
    ) -> CompletionOutcome | None:
        return completion_stream


def test_completion_client_plain_matches_litellm() -> None:
    with CompletionServer() as server:
        client = LiteLLM(api_key=GOOD_KEY, base_url=server.url)
        completions = client.chat.completions
        assert isinstance(completions, Completions)
        response = model_response(completions.create(messages=MESSAGES, model=GOOD_MODEL))
        reference = litellm_plain(
            model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY
        )
    assert normalized(response.model_dump()) == normalized(reference)
    assert client.params["api_key"] == GOOD_KEY
    assert client.params["base_url"] == server.url
    assert client.params["timeout"] == CLIENT_TIMEOUT
    assert client.params["max_retries"] is None
    assert "self" not in client.params


def test_completion_client_reads_num_retries_at_construction(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(betterlite, "num_retries", CONFIGURED_RETRIES)
    client = LiteLLM(api_key=GOOD_KEY)
    assert client.params["max_retries"] == CONFIGURED_RETRIES


def test_completion_client_async_branch_matches_litellm() -> None:
    with CompletionServer() as server:
        params: dict[str, JsonValue] = {
            "api_key": GOOD_KEY, "base_url": server.url, "acompletion": True,
        }
        chat = Chat(params, None)
        assert "acompletion" not in params
        completions = chat.completions
        assert isinstance(completions, AsyncCompletions)
        response = model_response(
            asyncio.run(await_outcome(completions.create(messages=MESSAGES, model=GOOD_MODEL)))
        )
        reference = asyncio.run(
            litellm_async_plain(
                model=GOOD_MODEL, messages=MESSAGES, api_base=server.url, api_key=GOOD_KEY
            )
        )
    assert normalized(response.model_dump()) == normalized(reference)


def test_completion_client_router_matches_litellm() -> None:
    ours_router = RecordingRouter()
    reference_router = RecordingRouter()
    ours_params: dict[str, JsonValue] = {"temperature": TEMPERATURE}
    reference_params: dict[str, JsonValue] = {"temperature": TEMPERATURE}
    ours = Completions(ours_params, ours_router)
    reference = litellm_module().Completions(reference_params, reference_router)
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


def test_completion_client_async_router_matches_litellm() -> None:
    ours_router = RecordingRouter()
    reference_router = RecordingRouter()
    ours = AsyncCompletions({"temperature": TEMPERATURE}, ours_router)
    reference = litellm_module().AsyncCompletions(
        {"temperature": TEMPERATURE}, reference_router
    )
    ours_outcome = asyncio.run(
        await_outcome(ours.create(messages=MESSAGES, model=GOOD_MODEL))
    )
    reference_outcome = asyncio.run(
        reference.create(messages=MESSAGES, model=GOOD_MODEL)
    )
    assert dump_of(ours_outcome) == dump_of(reference_outcome)
    assert ours_router.calls == reference_router.calls


def test_completion_retries_sync_matches_litellm() -> None:
    ours_counter = Counter()
    ours_error = raised(
        lambda: completion_with_retries(
            model=GOOD_MODEL, messages=MESSAGES, num_retries=SYNC_RETRIES,
            original_function=ours_counter.failing,
        ),
        ValueError,
    )
    reference_counter = Counter()
    reference_error = raised(
        lambda: litellm_module().completion_with_retries(
            model=GOOD_MODEL, messages=MESSAGES, num_retries=SYNC_RETRIES,
            original_function=reference_counter.failing,
        ),
        ValueError,
    )
    assert len(ours_counter.calls) == SYNC_RETRIES
    assert ours_counter.calls == reference_counter.calls
    assert str(ours_error) == str(reference_error)


async def ours_async_retries(counter: Counter) -> CompletionOutcome:
    return await acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=ASYNC_RETRIES,
        original_function=counter.failing,
    )


async def litellm_async_retries(counter: Counter) -> object:
    return await litellm_module().acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=ASYNC_RETRIES,
        original_function=counter.failing,
    )


def test_completion_retries_async_matches_litellm() -> None:
    ours_counter = Counter()
    ours_error = raised(lambda: asyncio.run(ours_async_retries(ours_counter)), ValueError)
    reference_counter = Counter()
    reference_error = raised(
        lambda: asyncio.run(litellm_async_retries(reference_counter)), ValueError
    )
    assert len(ours_counter.calls) == ASYNC_RETRIES
    assert ours_counter.calls == reference_counter.calls
    assert str(ours_error) == str(reference_error)


async def keep_outcome(keeper: Keeper) -> CompletionOutcome:
    return await acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=MIN_ATTEMPTS,
        original_function=keeper.keep,
    )


async def litellm_keep_outcome(keeper: Keeper) -> object:
    return await litellm_module().acompletion_with_retries(
        model=GOOD_MODEL, messages=MESSAGES, num_retries=MIN_ATTEMPTS,
        original_function=keeper.keep,
    )


def test_completion_retries_async_keeps_a_sync_outcome() -> None:
    ours = Keeper()
    assert asyncio.run(keep_outcome(ours)) is ours.response
    reference = Keeper()
    assert asyncio.run(litellm_keep_outcome(reference)) is reference.response


def test_completion_retries_helpers() -> None:
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


def test_completion_config_matches_litellm(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    with CompletionServer() as server:
        config: dict[str, JsonValue] = {
            "model": GOOD_MODEL, "messages": MESSAGES,
            "api_base": server.url, "api_key": GOOD_KEY,
        }
        config_file = tmp_path / CONFIG_NAME
        config_file.write_text(json.dumps(config))
        monkeypatch.setattr(betterlite, "config_path", str(config_file))
        litellm_ref = litellm_module()
        monkeypatch.setattr(litellm_ref, "config_path", str(config_file))
        ours = dump_of(config_completion())
        reference = dump_of(litellm_ref.config_completion())
    assert normalized(ours) == normalized(reference)


def test_completion_config_without_path_matches_litellm(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(betterlite, "config_path", None)
    litellm_ref = litellm_module()
    monkeypatch.setattr(litellm_ref, "config_path", None)
    ours = raised(lambda: config_completion(), ValueError)
    reference = raised(lambda: litellm_ref.config_completion(), ValueError)
    assert str(ours) == str(reference) == CONFIG_MESSAGE


def test_completion_adapter_translates_like_litellm(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    with CompletionServer() as server:
        ours_adapter = FakeAdapter()
        reference_adapter = FakeAdapter()
        monkeypatch.setattr(
            betterlite, "adapters", [{"id": ADAPTER_ID, "adapter": ours_adapter}]
        )
        litellm_ref = litellm_module()
        monkeypatch.setattr(
            litellm_ref, "adapters", [{"id": ADAPTER_ID, "adapter": reference_adapter}]
        )
        ours = dump_of(
            adapter_completion(
                adapter_id=ADAPTER_ID, model=UNKNOWN_MODEL, messages=MESSAGES,
                api_base=server.url, api_key=GOOD_KEY,
            )
        )
        reference = dump_of(
            litellm_ref.adapter_completion(
                adapter_id=ADAPTER_ID, model=UNKNOWN_MODEL, messages=MESSAGES,
                api_base=server.url, api_key=GOOD_KEY,
            )
        )
    assert normalized(ours) == normalized(reference)
    assert ours_adapter.input_calls == ADAPTER_CALLS
    assert ours_adapter.output_calls == ADAPTER_CALLS
    assert reference_adapter.input_calls == ADAPTER_CALLS
    assert reference_adapter.output_calls == ADAPTER_CALLS


async def ours_adapt_async(server_url: str) -> object:
    return await aadapter_completion(
        adapter_id=ADAPTER_ID, model=UNKNOWN_MODEL, messages=MESSAGES,
        api_base=server_url, api_key=GOOD_KEY,
    )


async def litellm_adapt_async(server_url: str) -> object:
    return await litellm_module().aadapter_completion(
        adapter_id=ADAPTER_ID, model=UNKNOWN_MODEL, messages=MESSAGES,
        api_base=server_url, api_key=GOOD_KEY,
    )


def test_completion_adapter_async_matches_litellm(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    with CompletionServer() as server:
        ours_adapter = FakeAdapter()
        reference_adapter = FakeAdapter()
        monkeypatch.setattr(
            betterlite, "adapters", [{"id": ADAPTER_ID, "adapter": ours_adapter}]
        )
        litellm_ref = litellm_module()
        monkeypatch.setattr(
            litellm_ref, "adapters", [{"id": ADAPTER_ID, "adapter": reference_adapter}]
        )
        ours = dump_of(asyncio.run(ours_adapt_async(server.url)))
        reference = dump_of(asyncio.run(litellm_adapt_async(server.url)))
    assert normalized(ours) == normalized(reference)
    assert ours_adapter.input_calls == ADAPTER_CALLS
    assert reference_adapter.input_calls == ADAPTER_CALLS


def test_completion_adapter_missing_matches_litellm(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(betterlite, "adapters", [])
    litellm_ref = litellm_module()
    monkeypatch.setattr(litellm_ref, "adapters", [])
    ours = raised(
        lambda: adapter_completion(adapter_id=ADAPTER_ID, model=GOOD_MODEL, messages=MESSAGES),
        ValueError,
    )
    reference = raised(
        lambda: litellm_ref.adapter_completion(
            adapter_id=ADAPTER_ID, model=GOOD_MODEL, messages=MESSAGES
        ),
        ValueError,
    )
    assert str(ours) == str(reference)
