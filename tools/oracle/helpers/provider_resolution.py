import os
from collections.abc import Callable
from contextlib import ExitStack
from typing import Final, TypeAlias, cast
from unittest import mock

import litellm
from litellm import constants
from litellm.litellm_core_utils import fallback_generalizations as generalizations
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

litellm.suppress_debug_info = True

# The sets get_llm_provider consults, rebuilt from the case catalog instead of the real cost map.
SET_NAMES: Final[tuple[str, ...]] = (
    "open_ai_chat_completion_models",
    "open_ai_text_completion_models",
    "cohere_models",
    "cohere_chat_models",
    "anthropic_models",
    "openrouter_models",
    "vertex_chat_models",
    "vertex_code_chat_models",
    "vertex_text_models",
    "vertex_code_text_models",
    "vertex_language_models",
    "vertex_embedding_models",
    "vertex_vision_models",
    "vertex_ai_image_models",
    "vertex_ai_video_models",
    "ai21_models",
    "ai21_chat_models",
    "nlp_cloud_models",
    "aleph_alpha_models",
    "bedrock_models",
    "bedrock_converse_models",
    "watsonx_models",
    "gradient_ai_models",
    "empower_models",
    "replicate_models",
    "maritalk_models",
    "petals_models",
    "baseten_models",
    "cohere_embedding_models",
    "open_ai_embedding_models",
    "bedrock_embedding_models",
)

_MODEL_COST: TypeAlias = dict[str, Json]


def _constant_seeds() -> dict[str, set[str]]:
    return {
        "replicate_models": cast(set[str], constants.replicate_models),
        "maritalk_models": set(litellm.maritalk_models),
        "petals_models": set(litellm.petals_models),
        "baseten_models": cast(set[str], constants.baseten_models),
        "cohere_embedding_models": cast(set[str], constants.cohere_embedding_models),
        "open_ai_embedding_models": cast(set[str], constants.open_ai_embedding_models),
        "bedrock_embedding_models": cast(set[str], constants.bedrock_embedding_models),
        "bedrock_converse_models": set(constants.BEDROCK_CONVERSE_MODELS),
        "empower_models": constants.empower_models,
    }


def _string(data: dict[str, Json], name: str) -> str | None:
    value = data.get(name)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _string_list(data: dict[str, Json], name: str) -> list[str]:
    value = data.get(name)
    if value is None:
        return []
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list of strings")
    names: list[str] = []
    for item in value:
        if not isinstance(item, str):
            raise ValueError(f"{name} must be a list of strings")
        names.append(item)
    return names


def _environment(data: dict[str, Json]) -> dict[str, str]:
    value = data.get("environment")
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError("environment must be an object")
    variables = {name: item for name, item in value.items() if isinstance(item, str)}
    if len(variables) != len(value):
        raise ValueError("environment values must be strings")
    return variables


def _catalog(data: dict[str, Json]) -> dict[str, Json]:
    value = data.get("catalog")
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise ValueError("catalog must be an object")
    return value


# Installs the case catalog's cost map into fresh sets, exactly as litellm.add_known_models would.
def _sets_from_catalog(catalog: _MODEL_COST) -> dict[str, set[str]]:
    seeds = _constant_seeds()
    fresh = {name: set(seeds.get(name, set())) for name in SET_NAMES}
    populate = cast(
        Callable[[_MODEL_COST], None],
        litellm._populate_provider_model_sets,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    with ExitStack() as stack:
        for name, values in fresh.items():
            stack.enter_context(mock.patch.object(litellm, name, values))
        populate(catalog)
    return fresh


def _install_generalizations(catalog: dict[str, Json]) -> list[Json] | None:
    rules: list[Json] | None = None
    block = catalog.get("fallback_generalizations")
    if isinstance(block, dict):
        declared = block.get("rules")
        if isinstance(declared, list):
            rules = declared
    get_rules = cast(
        Callable[[], list[Json]],
        generalizations.get_fallback_generalization_rules,
    )
    set_rules = cast(
        Callable[[list[Json] | None], None],
        generalizations.set_fallback_generalizations,
    )
    previous = get_rules()
    set_rules(rules)
    return previous


def run(data: dict[str, Json]) -> Json:
    model = _string(data, "model")
    if model is None:
        raise ValueError("model must be a string")
    custom_llm_provider = _string(data, "custom_llm_provider")
    api_base = _string(data, "api_base")
    api_key = _string(data, "api_key")
    environment = _environment(data)
    catalog = _catalog(data)
    known_model_names = _string_list(data, "known_model_names")
    custom_providers = _string_list(data, "custom_providers")

    installed = _sets_from_catalog(catalog)
    previous_rules = _install_generalizations(catalog)
    registered = [{"provider": name, "custom_handler": None} for name in custom_providers]
    try:
        with ExitStack() as stack:
            for name, values in installed.items():
                stack.enter_context(mock.patch.object(litellm, name, values))
            stack.enter_context(mock.patch.dict(os.environ, environment, clear=True))
            stack.enter_context(
                mock.patch.object(litellm, "model_list_set", set(known_model_names))
            )
            stack.enter_context(
                mock.patch.object(litellm, "custom_provider_map", registered)
            )
            resolved_model, provider, dynamic_api_key, resolved_base = get_llm_provider(
                model=model,
                custom_llm_provider=custom_llm_provider,
                api_base=api_base,
                api_key=api_key,
            )
    finally:
        set_rules = cast(
            Callable[[list[Json] | None], None],
            generalizations.set_fallback_generalizations,
        )
        set_rules(previous_rules)

    return {
        "model": resolved_model,
        "provider": provider,
        "api_base": resolved_base,
        "api_key": dynamic_api_key,
    }
