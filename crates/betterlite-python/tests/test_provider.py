from collections.abc import Callable
from typing import Final, TypeAlias

import litellm
import pytest
from litellm.litellm_core_utils.get_llm_provider_logic import (
    declared_authenticating_provider as their_declared_provider,
    handle_anthropic_text_model_custom_llm_provider as their_anthropic_handler,
    handle_cohere_chat_model_custom_llm_provider as their_cohere_handler,
    inferred_provider as their_inferred_provider,
    is_registered_custom_provider as their_registered_provider,
)
from litellm.types.router import GenericLiteLLMParams as TheirParams
from pydantic import JsonValue

import betterlite
from betterlite._provider import _get_openai_compatible_provider_info as our_compatible_info
from betterlite._router_types_params import GenericLiteLLMParams
from betterlite.exceptions import BadRequestError

PairHandler: TypeAlias = Callable[[str, str | None], tuple[str, str | None]]
CompatibleTuple: TypeAlias = tuple[str, str, str | None, str | None]
CompatibleCase: TypeAlias = tuple[str, str | None, str | None, str | None, CompatibleTuple]
MODELS: Final[list[str]] = [
    "gpt-4",
    "azure/gpt-4o",
    "azure/command-r-plus",
    "groq/llama3-8b-8192",
    "openrouter/x/y",
    "anthropic/claude-2",
    "command-r-plus-08-2024",
    "mistral/mistral-large-latest",
]
DECLARED: Final[list[tuple[str, str]]] = [
    ("llama3-8b-8192", "groq"),
    ("gpt-4o", "openai"),
    ("gpt-4", "azure"),
    ("command-r-plus-08-2024", "cohere"),
    ("claude-2", "anthropic"),
    ("claude-3-5-sonnet-20240620", "anthropic"),
]
API_BASES: Final[list[str]] = [
    "https://api.groq.com/openai/v1",
    "http://api.groq.com/openai/v1",
    "api.groq.com/openai/v1",
    "https://api.perplexity.ai",
    "api.deepseek.com/v1",
    "https://api.mistral.ai/v1",
    "api.endpoints.anyscale.com/v1",
    "https://api.getnadir.com/v1",
    "http://api.getnadir.com/v1",
    "https://api.cerebras.ai/v1",
]
HOSTILE: Final = "https://attacker.com/api.groq.com/openai/v1"
GROQ_BASE: Final = "https://api.groq.com/openai/v1"
PARAMS: Final[dict[str, str]] = {
    "custom_llm_provider": "groq",
    "api_base": "https://example/v1",
    "api_key": "sk-params",
}
# LiteLLM's recorded _get_openai_compatible_provider_info answers, and the public
# get_llm_provider call that reaches the same function for every known provider.
COMPATIBLE: Final[list[CompatibleCase]] = [
    ("groq/llama3-8b-8192", None, None, None, ("llama3-8b-8192", "groq", None, GROQ_BASE)),
    (
        "groq/llama3-8b-8192",
        "https://x/v1",
        "sk-x",
        None,
        ("llama3-8b-8192", "groq", "sk-x", "https://x/v1"),
    ),
    (
        "anyscale/foo",
        None,
        None,
        None,
        ("foo", "anyscale", None, "https://api.endpoints.anyscale.com/v1"),
    ),
    ("deepseek/foo", None, None, None, ("foo", "deepseek", None, "https://api.deepseek.com/beta")),
    ("azure/command-r-plus", None, None, None, ("command-r-plus", "azure", None, None)),
    ("openai/gpt-4", None, None, None, ("gpt-4", "openai", None, None)),
    (
        "mistral/mistral-large-latest",
        None,
        None,
        None,
        ("mistral-large-latest", "mistral", None, "https://api.mistral.ai/v1"),
    ),
    ("jina_ai/embed", None, None, None, ("embed", "jina_ai", None, "https://api.jina.ai/v1")),
    (
        "openrouter/anthropic/claude-3.5-sonnet",
        None,
        None,
        None,
        ("anthropic/claude-3.5-sonnet", "openrouter", None, None),
    ),
    ("mystery/model", None, None, None, ("model", "mystery", None, None)),
    ("mystery/model", "https://y/v1", "sk-a", "sk-d", ("model", "mystery", "sk-d", "https://y/v1")),
]
HANDLER_INPUTS: Final[list[tuple[str, str | None]]] = [
    ("cohere/command-r", None),
    ("cohere/command-r-plus-08-2024", None),
    ("command-r-plus-08-2024", "cohere"),
    ("command-r", "cohere"),
    ("anthropic/claude-2", None),
    ("claude-2", "anthropic"),
    ("anthropic/claude-3-5-sonnet", None),
    ("gpt-4", "groq"),
]
HANDLERS: Final[list[tuple[PairHandler, PairHandler]]] = [
    (their_cohere_handler, betterlite.handle_cohere_chat_model_custom_llm_provider),
    (their_anthropic_handler, betterlite.handle_anthropic_text_model_custom_llm_provider),
]
DECLARED_PAIRS: Final[list[tuple[str | None, str | None]]] = [
    (None, None),
    ("chatgpt/x", None),
    (None, "chatgpt"),
    ("github_copilot/x", None),
    ("gpt-4", None),
    ("x", "github_copilot"),
    ("x", "openai"),
]
INFERRED: Final[list[str | None]] = [
    None,
    "",
    "gpt-4",
    "azure/gpt-4o",
    "unknown-model-xyz",
    "chatgpt/x",
    "github_copilot/foo",
    "command-r-plus-08-2024",
]
UNLISTED: Final[list[str]] = ["claude-3-5-sonnet-20240620", "unknown-model-xyz"]
ENV_KEYS: Final[list[str]] = [
    "ANYSCALE_API_KEY",
    "GROQ_API_KEY",
    "JINA_AI_API_KEY",
    "MISTRAL_API_KEY",
]
@pytest.mark.parametrize("model", MODELS)
def test_lookup_matches_litellm_for_the_fixed_model_list(model: str) -> None:
    assert betterlite.get_llm_provider(model=model) == litellm.get_llm_provider(model=model)


@pytest.mark.parametrize(("model", "provider"), DECLARED)
def test_lookup_matches_litellm_for_a_declared_provider(model: str, provider: str) -> None:
    theirs = litellm.get_llm_provider(model=model, custom_llm_provider=provider)
    assert betterlite.get_llm_provider(model=model, custom_llm_provider=provider) == theirs


@pytest.mark.parametrize("api_base", API_BASES)
def test_lookup_matches_litellm_for_each_endpoint_api_base(api_base: str) -> None:
    ours = betterlite.get_llm_provider(model="gpt-4", api_base=api_base)
    assert ours == litellm.get_llm_provider(model="gpt-4", api_base=api_base)
    assert ours[3] == api_base


def test_lookup_endpoint_api_base_reads_the_provider_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GROQ_API_KEY", "sk-test-groq")
    ours = betterlite.get_llm_provider(model="gpt-4", api_base=GROQ_BASE)
    assert ours == litellm.get_llm_provider(model="gpt-4", api_base=GROQ_BASE)
    assert ours[2] == "sk-test-groq"


def test_lookup_litellm_params_fill_only_when_all_three_are_none() -> None:
    ours = betterlite.get_llm_provider(
        model="llama3-8b-8192", litellm_params=GenericLiteLLMParams(**PARAMS)
    )
    theirs = litellm.get_llm_provider(model="llama3-8b-8192", litellm_params=TheirParams(**PARAMS))
    assert ours == theirs


def test_lookup_reads_a_later_custom_provider_map(monkeypatch: pytest.MonkeyPatch) -> None:
    entries: list[JsonValue] = [{"provider": "myprovider", "custom_handler": None}]
    monkeypatch.setattr(betterlite, "custom_provider_map", entries)
    monkeypatch.setattr(litellm, "custom_provider_map", entries)
    ours = betterlite.get_llm_provider(model="myprovider/model-x")
    assert ours == litellm.get_llm_provider(model="myprovider/model-x")
    assert ours == ("model-x", "myprovider", None, None)


@pytest.mark.parametrize("name", [None, "myprovider", "other"])
def test_lookup_registered_custom_provider_follows_a_later_assignment(
    monkeypatch: pytest.MonkeyPatch, name: str | None
) -> None:
    entries: list[JsonValue] = [{"provider": "myprovider"}]
    monkeypatch.setattr(betterlite, "custom_provider_map", entries)
    monkeypatch.setattr(litellm, "custom_provider_map", entries)
    assert betterlite.is_registered_custom_provider(name) == their_registered_provider(name)
    assert betterlite.is_registered_custom_provider(name) is (name == "myprovider")


@pytest.mark.parametrize(("model", "provider"), DECLARED_PAIRS)
def test_lookup_declared_authenticating_provider_matches_litellm(
    model: str | None, provider: str | None
) -> None:
    assert betterlite.declared_authenticating_provider(model, provider) == (
        their_declared_provider(model, provider)
    )


@pytest.mark.parametrize("model", INFERRED)
def test_inferred_provider_matches_litellm(model: str | None) -> None:
    assert betterlite.inferred_provider(model) == their_inferred_provider(model)


def test_inferred_provider_returns_the_declaration_first() -> None:
    assert betterlite.inferred_provider("github_copilot/foo") == "github_copilot"
    assert betterlite.inferred_provider("chatgpt/x") == "chatgpt"
    assert betterlite.inferred_provider("") is None
    assert betterlite.inferred_provider("unknown-model-xyz") is None


@pytest.mark.parametrize("case", COMPATIBLE)
def test_openai_compatible_info_matches_litellm(
    monkeypatch: pytest.MonkeyPatch, case: CompatibleCase
) -> None:
    model, api_base, api_key, dynamic_api_key, expected = case
    for name in ENV_KEYS:
        monkeypatch.delenv(name, raising=False)
    assert our_compatible_info(model, api_base, api_key, dynamic_api_key) == expected
    if dynamic_api_key is None and model.split("/", 1)[0] in litellm.provider_list:
        assert litellm.get_llm_provider(model=model, api_base=api_base, api_key=api_key) == expected


@pytest.mark.parametrize(("theirs", "ours"), HANDLERS)
@pytest.mark.parametrize(("model", "provider"), HANDLER_INPUTS)
def test_openai_compatible_handlers_match_litellm(
    theirs: PairHandler, ours: PairHandler, model: str, provider: str | None
) -> None:
    assert ours(model, provider) == theirs(model, provider)


@pytest.mark.parametrize("model", UNLISTED)
def test_errors_unlisted_models_raise_like_litellm(model: str) -> None:
    with pytest.raises(litellm.exceptions.BadRequestError) as expected:
        litellm.get_llm_provider(model=model)
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(model=model)
    assert str(actual.value) == str(expected.value)


def test_errors_none_model_matches_litellm() -> None:
    expected = (
        "litellm.BadRequestError: GetLLMProvider Exception - model parameter is required but "
        "was None. Please provide a valid model name.\n\noriginal model: None"
    )
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(model=None)
    assert str(actual.value) == expected


def test_errors_litellm_params_are_ignored_when_one_argument_is_set() -> None:
    with pytest.raises(litellm.exceptions.BadRequestError) as expected:
        litellm.get_llm_provider(
            model="llama3-8b-8192",
            custom_llm_provider="",
            litellm_params=TheirParams(**PARAMS),
        )
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(
            model="llama3-8b-8192",
            custom_llm_provider="",
            litellm_params=GenericLiteLLMParams(**PARAMS),
        )
    assert str(actual.value) == str(expected.value)


def test_hostile_api_base_does_not_select_the_groq_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GROQ_API_KEY", "sk-test-groq")
    ours = betterlite.get_llm_provider(model="gpt-4", api_base=HOSTILE)
    assert ours == litellm.get_llm_provider(model="gpt-4", api_base=HOSTILE)
    assert ours[1] != "groq"
    assert ours[2] is None


def test_hostile_api_base_with_an_unknown_model_raises_like_litellm(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("GROQ_API_KEY", "sk-test-groq")
    with pytest.raises(litellm.exceptions.BadRequestError) as expected:
        litellm.get_llm_provider(model="unknown-model-xyz", api_base=HOSTILE)
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(model="unknown-model-xyz", api_base=HOSTILE)
    assert str(actual.value) == str(expected.value)
