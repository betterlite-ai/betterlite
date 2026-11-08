from collections.abc import Callable, Mapping, Sequence
from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
import litellm.main as litellm_main
from litellm.types.llms.openai import OpenAIWebSearchOptions
from pydantic import BaseModel, ConfigDict, Field, JsonValue

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class BridgeRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    custom_llm_provider: str
    web_search_options: JsonValue = None
    tools: list[dict[str, JsonValue]] = Field(default_factory=list)
    reasoning_effort: str | dict[str, JsonValue] | None = None
    reasoning_summary: JsonValue = None
    api_base: str | None = None
    route_all_chat_openai_to_responses: bool = False


class FoundryFacts(BaseModel):
    model_config = ConfigDict(extra="forbid")

    supports_native_responses: bool = False
    chat_rejects_function_tools: bool = False
    model: str = ""
    api_base: str | None = None
    effort: str | None = None


class GptFacts(BaseModel):
    model_config = ConfigDict(extra="forbid")

    five: bool = False
    search: bool = False
    model: str = ""


class Capabilities(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model_info: dict[str, JsonValue] = Field(default_factory=dict)
    model_info_fails: bool = False
    resolved_openai_api_base: str
    foundry: FoundryFacts = Field(default_factory=FoundryFacts)
    gpt: GptFacts = Field(default_factory=GptFacts)
    gpt_5_4_plus: bool = False


class Case(BaseModel):
    model_config = ConfigDict(extra="forbid")

    request: BridgeRequest
    capabilities: Capabilities


class ModelInfoLookup:
    def __init__(self, model_info: dict[str, JsonValue], *, fails: bool) -> None:
        self.model_info = model_info
        self.fails = fails

    def __call__(self, **kwargs: object) -> dict[str, JsonValue]:
        if self.fails:
            raise ValueError(f"model info is not known for {kwargs.get('model')}")
        return dict(self.model_info)


class ResolvedBase:
    def __init__(self, api_base: str) -> None:
        self.api_base = api_base

    def __call__(self, api_base: str | None) -> str:
        return self.api_base if api_base is None else api_base


class FoundrySupports:
    def __init__(self, *, supported: bool, model: str, api_base: str | None) -> None:
        self.supported = supported
        self.model = model
        self.api_base = api_base

    def __call__(self, model: str | None, api_base: str | None) -> bool:
        return self.supported and model == self.model and api_base == self.api_base


class FoundryRejects:
    def __init__(self, *, rejects: bool, model: str, effort: str | None) -> None:
        self.rejects = rejects
        self.model = model
        self.effort = effort

    def __call__(
        self, model: str, reasoning_effort: str | Mapping[str, object] | None
    ) -> bool:
        return (
            self.rejects
            and self.effort is not None
            and model == self.model
            and reasoning_effort == self.effort
        )


class GptFiveNames:
    def __init__(self, *, facts: GptFacts, four_plus: bool) -> None:
        self.facts = facts
        self.four_plus = four_plus

    def is_model_gpt_5_model(self, model: str) -> bool:
        return self.facts.five and model == self.facts.model

    def is_model_gpt_5_4_plus_model(self, model: str) -> bool:
        return self.four_plus and model == self.facts.model

    def is_model_gpt_5_search_model(self, model: str) -> bool:
        return self.facts.search and model == self.facts.model


def run(input_value: dict[str, Json]) -> dict[str, Json]:
    case = Case.model_validate(input_value)
    request = case.request
    facts = case.capabilities
    bridge_check = cast(
        Callable[..., tuple[dict[str, Json], str]],
        litellm_main.responses_api_bridge_check,
    )
    with (
        patch.object(
            litellm_main,
            "_get_model_info_helper",
            ModelInfoLookup(facts.model_info, fails=facts.model_info_fails),
        ),
        patch.object(
            litellm_main,
            "_resolve_openai_api_base",
            ResolvedBase(facts.resolved_openai_api_base),
        ),
        patch.object(
            litellm_main,
            "azure_ai_supports_native_responses",
            FoundrySupports(
                supported=facts.foundry.supports_native_responses,
                model=facts.foundry.model,
                api_base=facts.foundry.api_base,
            ),
        ),
        patch.object(
            litellm_main,
            "foundry_chat_rejects_function_tools_while_reasoning",
            FoundryRejects(
                rejects=facts.foundry.chat_rejects_function_tools,
                model=facts.foundry.model,
                effort=facts.foundry.effort,
            ),
        ),
        patch.object(
            litellm_main,
            "OpenAIGPT5Config",
            GptFiveNames(facts=facts.gpt, four_plus=facts.gpt_5_4_plus),
        ),
    ):
        litellm.route_all_chat_openai_to_responses = (
            request.route_all_chat_openai_to_responses
        )
        model_info, model = bridge_check(
            request.model,
            request.custom_llm_provider,
            cast(OpenAIWebSearchOptions | None, request.web_search_options),
            cast(Sequence[Mapping[str, object]] | None, request.tools),
            cast(str | Mapping[str, object] | None, request.reasoning_effort),
            request.reasoning_summary,
            request.api_base,
        )
    return {"model_info": cast(Json, model_info), "model": model}
