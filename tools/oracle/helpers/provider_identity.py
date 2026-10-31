from typing import Literal, TypeAlias, TypedDict
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils import get_llm_provider_logic as provider_logic

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_is_non_openai_azure_model = provider_logic._is_non_openai_azure_model  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_is_azure_claude_model = provider_logic._is_azure_claude_model  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
_endpoint_matches_api_base = provider_logic._endpoint_matches_api_base  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
handle_cohere_chat_model_custom_llm_provider = (
    provider_logic.handle_cohere_chat_model_custom_llm_provider
)
handle_anthropic_text_model_custom_llm_provider = (
    provider_logic.handle_anthropic_text_model_custom_llm_provider
)


class CatalogEntry(TypedDict):
    litellm_provider: str


class ExplicitProviderInput(TypedDict):
    check: Literal["explicit_provider"]
    model: str
    custom_llm_provider: str | None
    configured_model_names: list[str]
    catalog: dict[str, CatalogEntry]


class CohereInput(TypedDict):
    check: Literal["cohere"]
    model: str
    custom_llm_provider: str | None
    catalog: dict[str, CatalogEntry]


class AnthropicTextInput(TypedDict):
    check: Literal["anthropic_text"]
    model: str
    custom_llm_provider: str | None


class AzureClaudeInput(TypedDict):
    check: Literal["azure_claude"]
    model: str


class OpenrouterInput(TypedDict):
    check: Literal["openrouter"]
    model: str
    custom_llm_provider: str | None


class EndpointInput(TypedDict):
    check: Literal["endpoint"]
    endpoint: str
    api_base: str


Input = (
    ExplicitProviderInput
    | CohereInput
    | AnthropicTextInput
    | AzureClaudeInput
    | OpenrouterInput
    | EndpointInput
)


def _catalog_sets(catalog: dict[str, CatalogEntry]) -> tuple[set[str], set[str]]:
    cohere_models = {
        name for name, entry in catalog.items() if entry["litellm_provider"] == "cohere_chat"
    }
    mistral_models = {
        name for name, entry in catalog.items() if entry["litellm_provider"] == "mistral"
    }
    return cohere_models, mistral_models


def _run_explicit_provider(input_value: ExplicitProviderInput) -> Json:
    model = input_value["model"]
    custom_llm_provider = input_value["custom_llm_provider"]
    configured_names = set(input_value["configured_model_names"])
    cohere_models, mistral_models = _catalog_sets(input_value["catalog"])
    with (
        patch.object(litellm, "cohere_chat_models", cohere_models),
        patch.object(litellm, "mistral_chat_models", mistral_models),
    ):
        if model.split("/", 1)[0] == "azure" and _is_non_openai_azure_model(model):
            return {"kind": "non_openai_azure", "model": model}
        model, custom_llm_provider = handle_cohere_chat_model_custom_llm_provider(
            model, custom_llm_provider
        )
    model, custom_llm_provider = handle_anthropic_text_model_custom_llm_provider(
        model, custom_llm_provider
    )
    if custom_llm_provider and model.split("/")[0] != custom_llm_provider:
        model = f"{custom_llm_provider}/{model}"
    if custom_llm_provider == "openrouter" and model.startswith("openrouter/"):
        remainder = model[len("openrouter/") :]
        if "/" in remainder:
            return {"kind": "openrouter_nested", "model": remainder}
        return {"kind": "openrouter_native", "model": model}
    prefix, slash, rest = model.partition("/")
    if not slash or prefix not in litellm.provider_list:
        return {"kind": "unresolved", "model": model, "custom_llm_provider": custom_llm_provider}
    return {
        "kind": "explicit_provider",
        "provider": prefix,
        "model": rest,
        "needs_provider_defaults": prefix not in configured_names,
    }


def _run_cohere(input_value: CohereInput) -> Json:
    cohere_models, mistral_models = _catalog_sets(input_value["catalog"])
    with (
        patch.object(litellm, "cohere_chat_models", cohere_models),
        patch.object(litellm, "mistral_chat_models", mistral_models),
    ):
        model, provider = handle_cohere_chat_model_custom_llm_provider(
            input_value["model"], input_value["custom_llm_provider"]
        )
    return [model, provider]


def _run_anthropic_text(input_value: AnthropicTextInput) -> Json:
    model, provider = handle_anthropic_text_model_custom_llm_provider(
        input_value["model"], input_value["custom_llm_provider"]
    )
    return [model, provider]


def _run_azure_claude(input_value: AzureClaudeInput) -> Json:
    return _is_azure_claude_model(input_value["model"])


def _run_openrouter(input_value: OpenrouterInput) -> Json:
    model = input_value["model"]
    custom_llm_provider = input_value["custom_llm_provider"]
    if custom_llm_provider == "openrouter" and model.startswith("openrouter/"):
        remainder = model[len("openrouter/") :]
        if "/" in remainder:
            return {"kind": "nested", "model": remainder}
        return {"kind": "native", "model": model}
    return {"kind": "not_applicable"}


def _run_endpoint(input_value: EndpointInput) -> Json:
    return _endpoint_matches_api_base(input_value["endpoint"], input_value["api_base"])


def run(input_value: Input) -> Json:
    if input_value["check"] == "explicit_provider":
        return _run_explicit_provider(input_value)
    if input_value["check"] == "cohere":
        return _run_cohere(input_value)
    if input_value["check"] == "anthropic_text":
        return _run_anthropic_text(input_value)
    if input_value["check"] == "azure_claude":
        return _run_azure_claude(input_value)
    if input_value["check"] == "openrouter":
        return _run_openrouter(input_value)
    return _run_endpoint(input_value)
