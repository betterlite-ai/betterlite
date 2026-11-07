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


class Capabilities(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model_info: dict[str, JsonValue] = Field(default_factory=dict)
    model_info_fails: bool = False
    resolved_openai_api_base: str
    foundry_supports_native_responses: bool = False
    foundry_chat_rejects_function_tools: bool = False
    is_gpt_5_model: bool = False
    is_gpt_5_4_plus_model: bool = False
    is_gpt_5_search_model: bool = False


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

    def __call__(self, _api_base: str | None) -> str:
        return self.api_base


class FoundrySupports:
    def __init__(self, *, supported: bool) -> None:
        self.supported = supported

    def __call__(self, _model: str | None, _api_base: str | None) -> bool:
        return self.supported


class FoundryRejects:
    def __init__(self, *, rejects: bool) -> None:
        self.rejects = rejects

    def __call__(
        self, _model: str, _reasoning_effort: str | Mapping[str, object] | None
    ) -> bool:
        return self.rejects


class GptFiveNames:
    def __init__(self, *, gpt_5: bool, gpt_5_4_plus: bool, gpt_5_search: bool) -> None:
        self.gpt_5 = gpt_5
        self.gpt_5_4_plus = gpt_5_4_plus
        self.gpt_5_search = gpt_5_search

    def is_model_gpt_5_model(self, _model: str) -> bool:
        return self.gpt_5

    def is_model_gpt_5_4_plus_model(self, _model: str) -> bool:
        return self.gpt_5_4_plus

    def is_model_gpt_5_search_model(self, _model: str) -> bool:
        return self.gpt_5_search


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
            FoundrySupports(supported=facts.foundry_supports_native_responses),
        ),
        patch.object(
            litellm_main,
            "foundry_chat_rejects_function_tools_while_reasoning",
            FoundryRejects(rejects=facts.foundry_chat_rejects_function_tools),
        ),
        patch.object(
            litellm_main,
            "OpenAIGPT5Config",
            GptFiveNames(
                gpt_5=facts.is_gpt_5_model,
                gpt_5_4_plus=facts.is_gpt_5_4_plus_model,
                gpt_5_search=facts.is_gpt_5_search_model,
            ),
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
