import os
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

import litellm
import litellm.constants
from litellm.llms.mistral.chat.transformation import MistralConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

TRANSFORMATION = (
    "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/"
    "litellm/llms/mistral/chat/transformation.py"
)
CONSTANTS = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L85"
MODEL_COST = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563"
PROVIDER_LOGIC = (
    "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/"
    "litellm/litellm_core_utils/get_llm_provider_logic.py#L158"
)


class Settings(TypedDict, total=False):
    temperature: int
    top_p: int
    max_tokens: int
    tools: list[Json]
    tool_choice: Literal["auto", "any", "none"]
    random_seed: int
    safe_prompt: bool
    response_format: dict[str, Json]
    stop: str | list[Json]


class Catalog(TypedDict, total=False):
    supports_reasoning: bool
    declared_efforts: list[str]


class OptionsStep(TypedDict):
    kind: Literal["options"]
    model: str
    max_depth: int
    params: NotRequired[list[tuple[str, Json]]]
    optional_params: NotRequired[dict[str, Json]]


class ProviderStep(TypedDict):
    kind: Literal["provider_info"]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    azure_api_base: NotRequired[str]
    azure_api_key: NotRequired[str]
    mistral_api_key: NotRequired[str]


class PromptStep(TypedDict):
    kind: Literal["prompt"]


class Input(TypedDict):
    steps: list[OptionsStep | ProviderStep | PromptStep]
    settings: NotRequired[Settings]
    catalog: NotRequired[Catalog]


def build_config(settings: Settings | None) -> MistralConfig:
    if settings is None:
        return MistralConfig()
    return MistralConfig(
        temperature=settings.get("temperature"),
        top_p=settings.get("top_p"),
        max_tokens=settings.get("max_tokens"),
        tools=settings.get("tools"),
        tool_choice=settings.get("tool_choice"),
        random_seed=settings.get("random_seed"),
        safe_prompt=settings.get("safe_prompt"),
        response_format=settings.get("response_format"),
        stop=settings.get("stop"),
    )


def install_catalog(model: str, catalog: Catalog | None) -> None:
    entry: dict[str, Json] = {"supports_reasoning": False}
    if catalog is not None:
        supports = catalog.get("supports_reasoning")
        if supports is not None:
            entry["supports_reasoning"] = supports
        declared = catalog.get("declared_efforts")
        if declared is not None:
            entry["reasoning_effort_levels"] = [*declared]
    litellm.model_cost[f"mistral/{model}"] = entry  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563


def options_value(
    config: MistralConfig, catalog: Catalog | None, step: OptionsStep
) -> Json:
    model = step["model"]
    install_catalog(model, catalog)
    litellm.constants.DEFAULT_MAX_RECURSE_DEPTH = step["max_depth"]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L85
    mapped_params: dict[str, Json] = {}
    params = step.get("params")
    if params is not None:
        mapped_params.update(params)
    mapped_optional: dict[str, Json] = {}
    optional = step.get("optional_params")
    if optional is not None:
        mapped_optional.update(optional)
    mapped = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/chat/transformation.py#L171
        "dict[str, Json]",
        config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/chat/transformation.py#L171
            mapped_params,
            mapped_optional,
            model,
            drop_params=False,
        ),
    )
    exported = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/chat/transformation.py#L107
        "dict[str, Json]",
        config.get_config(),  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/chat/transformation.py#L107
    )
    supported: list[Json] = [*config.get_supported_openai_params(model)]
    return {
        "model": model,
        "config": exported,
        "supported": supported,
        "mapped": mapped,
    }


def provider_value(step: ProviderStep) -> Json:
    os.environ.clear()
    azure_base = step.get("azure_api_base")
    if azure_base is not None:
        os.environ["MISTRAL_AZURE_API_BASE"] = azure_base
    azure_key = step.get("azure_api_key")
    if azure_key is not None:
        os.environ["MISTRAL_AZURE_API_KEY"] = azure_key
    mistral_key = step.get("mistral_api_key")
    if mistral_key is not None:
        os.environ["MISTRAL_API_KEY"] = mistral_key
    resolved: tuple[str, str, str | None, str | None] = litellm.get_llm_provider(
        model="mistral/mistral-small-latest",
        api_base=step.get("api_base"),
        api_key=step.get("api_key"),
    )
    return {"api_base": resolved[3], "api_key": resolved[2]}


def prompt_value() -> Json:
    prompt = MistralConfig._get_mistral_reasoning_system_prompt()  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/mistral/chat/transformation.py#L145
    return {"prompt": prompt}


def run(data: Input) -> Json:
    config = build_config(data.get("settings"))
    catalog = data.get("catalog")
    results: list[Json] = []
    for step in data["steps"]:
        if step["kind"] == "options":
            results.append(options_value(config, catalog, step))
        elif step["kind"] == "provider_info":
            results.append(provider_value(step))
        else:
            results.append(prompt_value())
    return results
