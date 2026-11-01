from typing import Literal, TypeAlias, TypedDict
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils import get_llm_provider_logic as provider_logic

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

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


def _normalize_declared_provider(
    model: str, custom_llm_provider: str | None
) -> tuple[str, str | None]:
    model, custom_llm_provider = handle_cohere_chat_model_custom_llm_provider(
        model, custom_llm_provider
    )
    model, custom_llm_provider = handle_anthropic_text_model_custom_llm_provider(
        model, custom_llm_provider
    )
    if custom_llm_provider and model.split("/", 1)[0] != custom_llm_provider:
        model = f"{custom_llm_provider}/{model}"
    return model, custom_llm_provider


def _run_explicit_provider(input_value: ExplicitProviderInput) -> Json:
    model = input_value["model"]
    custom_llm_provider = input_value["custom_llm_provider"]
    configured_names = set(input_value["configured_model_names"])
    cohere_models, mistral_models = _catalog_sets(input_value["catalog"])
    reached_openai_compatible = False

    def _record_reached(
        model: str,
        api_base: str | None,
        api_key: str | None,  # noqa: ARG001
        dynamic_api_key: str | None,
        litellm_params: object | None = None,  # noqa: ARG001
    ) -> tuple[str, str, str | None, str | None]:
        nonlocal reached_openai_compatible
        reached_openai_compatible = True
        prefix, rest = model.split("/", 1)
        return rest, prefix, dynamic_api_key, api_base

    with (
        patch.object(litellm, "cohere_chat_models", cohere_models),
        patch.object(litellm, "mistral_chat_models", mistral_models),
        patch.object(litellm, "model_list_set", configured_names),
        patch.object(
            provider_logic, "_get_openai_compatible_provider_info", _record_reached
        ),
    ):
        normalized_model, normalized_provider = _normalize_declared_provider(
            model, custom_llm_provider
        )
        try:
            resolved_model, resolved_provider, _key, _base = (
                provider_logic.get_llm_provider(model, custom_llm_provider)
            )
        except litellm.exceptions.BadRequestError:
            return {
                "kind": "unresolved",
                "model": normalized_model,
                "custom_llm_provider": normalized_provider,
            }

    model_prefix = model.split("/", 1)[0]
    if resolved_provider == "openai" and resolved_model == model and model_prefix == "azure":
        return {"kind": "non_openai_azure", "model": resolved_model}
    if reached_openai_compatible:
        return {
            "kind": "explicit_provider",
            "provider": resolved_provider,
            "model": resolved_model,
            "needs_provider_defaults": True,
        }
    if custom_llm_provider == "openrouter" and model.startswith("openrouter/"):
        is_native = resolved_model.startswith("openrouter/")
        kind = "openrouter_native" if is_native else "openrouter_nested"
        return {"kind": kind, "model": resolved_model}
    if resolved_model != normalized_model:
        return {
            "kind": "explicit_provider",
            "provider": resolved_provider,
            "model": resolved_model,
            "needs_provider_defaults": False,
        }
    return {
        "kind": "unresolved",
        "model": normalized_model,
        "custom_llm_provider": normalized_provider,
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
    if custom_llm_provider != "openrouter" or not model.startswith("openrouter/"):
        return {"kind": "not_applicable"}
    resolved_model, _provider, _dynamic_api_key, _api_base = provider_logic.get_llm_provider(
        model, custom_llm_provider
    )
    if resolved_model.startswith("openrouter/"):
        return {"kind": "native", "model": resolved_model}
    return {"kind": "nested", "model": resolved_model}


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
