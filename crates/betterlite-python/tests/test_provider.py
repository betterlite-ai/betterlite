from collections.abc import Callable, Iterator
from typing import Final, TypeAlias

import litellm
import openai
import pytest
from litellm.litellm_core_utils import get_llm_provider_logic
from litellm.llms.custom_llm import CustomLLM
from litellm.types.router import GenericLiteLLMParams as TheirParams

import betterlite
from betterlite._provider import _get_openai_compatible_provider_info as our_compatible_info
from betterlite._router_types_params import GenericLiteLLMParams
from betterlite.exceptions import BadRequestError

PairHandler: TypeAlias = Callable[[str, str | None], tuple[str, str | None]]
FourTuple: TypeAlias = tuple[str, str, str | None, str | None]
CompatibleCase: TypeAlias = tuple[str, str | None, str | None, str | None, FourTuple]
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
# LiteLLM's recorded compatible-provider answers, and the public call that reaches them
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
    (
        get_llm_provider_logic.handle_cohere_chat_model_custom_llm_provider,
        betterlite.handle_cohere_chat_model_custom_llm_provider,
    ),
    (
        get_llm_provider_logic.handle_anthropic_text_model_custom_llm_provider,
        betterlite.handle_anthropic_text_model_custom_llm_provider,
    ),
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
def test_lookup_matches_litellm(model: str) -> None:
    theirs = get_llm_provider_logic.get_llm_provider(model=model)
    assert betterlite.get_llm_provider(model=model) == theirs


@pytest.mark.parametrize(("model", "provider"), DECLARED)
def test_declared_lookup(model: str, provider: str) -> None:
    theirs = get_llm_provider_logic.get_llm_provider(model=model, custom_llm_provider=provider)
    assert betterlite.get_llm_provider(model=model, custom_llm_provider=provider) == theirs


@pytest.mark.parametrize("api_base", API_BASES)
def test_endpoint_lookup(api_base: str) -> None:
    ours = betterlite.get_llm_provider(model="gpt-4", api_base=api_base)
    theirs = get_llm_provider_logic.get_llm_provider(model="gpt-4", api_base=api_base)
    assert ours == theirs
    assert ours[3] == api_base


def test_lookup_endpoint_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GROQ_API_KEY", "sk-test-groq")
    ours = betterlite.get_llm_provider(model="gpt-4", api_base=GROQ_BASE)
    theirs = get_llm_provider_logic.get_llm_provider(model="gpt-4", api_base=GROQ_BASE)
    assert ours == theirs
    assert ours[2] == "sk-test-groq"


def test_lookup_params_fill() -> None:
    ours = betterlite.get_llm_provider(
        model="llama3-8b-8192",
        litellm_params=GenericLiteLLMParams(
            custom_llm_provider="groq", api_base="https://example/v1", api_key="sk-params"
        ),
    )
    theirs = get_llm_provider_logic.get_llm_provider(
        model="llama3-8b-8192",
        litellm_params=TheirParams(
            custom_llm_provider="groq", api_base="https://example/v1", api_key="sk-params"
        ),
    )
    assert ours == theirs


@pytest.fixture()
def custom_maps() -> Iterator[list[str]]:
    betterlite.custom_provider_map = [{"provider": "myprovider"}]
    litellm.custom_provider_map = [{"provider": "myprovider", "custom_handler": CustomLLM()}]
    yield ["myprovider"]
    betterlite.custom_provider_map = []
    litellm.custom_provider_map = []


def test_lookup_later_provider_map(custom_maps: list[str]) -> None:
    provider = custom_maps[0]
    ours = betterlite.get_llm_provider(model=f"{provider}/model-x")
    theirs = get_llm_provider_logic.get_llm_provider(model=f"{provider}/model-x")
    assert ours == theirs


def test_inferred_custom_map(custom_maps: list[str]) -> None:
    provider = custom_maps[0]
    theirs = get_llm_provider_logic.inferred_provider(f"{provider}/model-x")
    assert betterlite.inferred_provider(f"{provider}/model-x") == theirs


@pytest.mark.parametrize("name", [None, "myprovider", "other"])
def test_lookup_registered_map(custom_maps: list[str], name: str | None) -> None:
    theirs = get_llm_provider_logic.is_registered_custom_provider(name)
    assert betterlite.is_registered_custom_provider(name) == theirs
    assert betterlite.is_registered_custom_provider(name) is (name in custom_maps)


@pytest.mark.parametrize(("model", "provider"), DECLARED_PAIRS)
def test_lookup_declared_auth(model: str | None, provider: str | None) -> None:
    theirs = get_llm_provider_logic.declared_authenticating_provider(model, provider)
    assert betterlite.declared_authenticating_provider(model, provider) == theirs


@pytest.mark.parametrize("model", INFERRED)
def test_inferred_matches(model: str | None) -> None:
    theirs = get_llm_provider_logic.inferred_provider(model)
    assert betterlite.inferred_provider(model) == theirs


def test_inferred_declared_first() -> None:
    assert betterlite.inferred_provider("github_copilot/foo") == "github_copilot"
    assert betterlite.inferred_provider("chatgpt/x") == "chatgpt"
    assert betterlite.inferred_provider("") is None
    assert betterlite.inferred_provider("unknown-model-xyz") is None


@pytest.mark.parametrize("case", COMPATIBLE)
def test_openai_compatible_info(monkeypatch: pytest.MonkeyPatch, case: CompatibleCase) -> None:
    model, api_base, api_key, dynamic_api_key, expected = case
    for name in ENV_KEYS:
        monkeypatch.delenv(name, raising=False)
    assert our_compatible_info(model, api_base, api_key, dynamic_api_key) == expected
    if dynamic_api_key is None and model.split("/", 1)[0] in litellm.provider_list:
        theirs = get_llm_provider_logic.get_llm_provider(
            model=model, api_base=api_base, api_key=api_key
        )
        assert theirs == expected


@pytest.mark.parametrize(("theirs", "ours"), HANDLERS)
@pytest.mark.parametrize(("model", "provider"), HANDLER_INPUTS)
def test_openai_compatible_helpers(
    theirs: PairHandler, ours: PairHandler, model: str, provider: str | None
) -> None:
    expected = theirs(model, provider)
    assert ours(model, provider) == expected


@pytest.mark.parametrize("model", UNLISTED)
def test_errors_unlisted_models(model: str) -> None:
    caught: list[str] = []
    try:
        get_llm_provider_logic.get_llm_provider(model=model)
    except openai.BadRequestError as error:
        caught.append(str(error))
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(model=model)
    assert caught == [str(actual.value)]


def test_errors_none_model() -> None:
    expected = (
        "litellm.BadRequestError: GetLLMProvider Exception - model parameter is required but "
        "was None. Please provide a valid model name.\n\noriginal model: None"
    )
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(model=None)
    assert str(actual.value) == expected


def test_errors_params_ignored() -> None:
    theirs_params = TheirParams(
        custom_llm_provider="groq", api_base="https://example/v1", api_key="sk-params"
    )
    ours_params = GenericLiteLLMParams(
        custom_llm_provider="groq", api_base="https://example/v1", api_key="sk-params"
    )
    caught: list[str] = []
    try:
        get_llm_provider_logic.get_llm_provider(
            model="llama3-8b-8192", custom_llm_provider="", litellm_params=theirs_params
        )
    except openai.BadRequestError as error:
        caught.append(str(error))
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(
            model="llama3-8b-8192", custom_llm_provider="", litellm_params=ours_params
        )
    assert caught == [str(actual.value)]


def test_hostile_base_no_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GROQ_API_KEY", "sk-test-groq")
    ours = betterlite.get_llm_provider(model="gpt-4", api_base=HOSTILE)
    theirs = get_llm_provider_logic.get_llm_provider(model="gpt-4", api_base=HOSTILE)
    assert ours == theirs
    assert ours[2] is None


def test_hostile_unknown_error(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GROQ_API_KEY", "sk-test-groq")
    with pytest.raises(openai.BadRequestError) as expected:
        get_llm_provider_logic.get_llm_provider(model="unknown-model-xyz", api_base=HOSTILE)
    with pytest.raises(BadRequestError) as actual:
        betterlite.get_llm_provider(model="unknown-model-xyz", api_base=HOSTILE)
    assert str(actual.value) == str(expected.value)
