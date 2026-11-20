from collections.abc import Sequence
from typing import Final, Literal, Protocol, TypedDict

import litellm
from litellm.exceptions import BadRequestError
from litellm.router import Router
from litellm.types.router import (
    DeploymentTypedDict,
    ModelGroupInfo,
    RouterModelGroupAliasItem,
)
from pydantic import JsonValue, TypeAdapter

ANSWER: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
DEPLOYMENT_INFO_METHOD: Final = "get_deployment_model_info"
SUPPORTED_PARAMS_FUNCTION: Final = "get_supported_openai_params"
LLM_PROVIDER_FUNCTION: Final = "get_llm_provider"


class Input(TypedDict):
    model_group: str
    model_list: list[DeploymentTypedDict]
    model_group_alias: dict[str, str | RouterModelGroupAliasItem]
    deployment_info: dict[str, dict[str, JsonValue]]
    model_cost: dict[str, dict[str, JsonValue]]
    parameters: dict[str, list[str] | None]
    lookup_error: bool
    provider_error: Literal["none", "bad_request", "other"]
    missing_ids: list[int]


class RouterStorage(Protocol):
    model_list: Sequence[dict[str, JsonValue]]


class FixedDeploymentInfo:
    def __init__(
        self, answers: dict[str, dict[str, JsonValue]], failing: bool
    ) -> None:
        self.answers = answers
        self.failing = failing

    def __call__(
        self, model_id: str, model_name: str
    ) -> dict[str, JsonValue] | None:
        _ = model_name
        if self.failing:
            raise AttributeError("the case asked for a failing deployment metadata lookup")
        return self.answers.get(model_id)


class FixedParameters:
    def __init__(self, answers: dict[str, list[str] | None]) -> None:
        self.answers = answers

    def __call__(
        self, model: str, custom_llm_provider: str | None = None
    ) -> list[str] | None:
        if custom_llm_provider is None:
            return self.answers.get(model)
        return self.answers.get(custom_llm_provider + "/" + model)


class FailingProvider:
    def __init__(self, bad_request: bool) -> None:
        self.bad_request = bad_request

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> tuple[str, str, str | None, str | None]:
        _ = custom_llm_provider, api_base, api_key
        if self.bad_request:
            raise BadRequestError(
                message="LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model="
                + model,
                model=model,
                llm_provider="",
            )
        raise RuntimeError("the case asked for a provider error")


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    router = Router(model_list=input_value["model_list"])
    router.model_group_alias = input_value["model_group_alias"]
    litellm.model_cost = dict(input_value["model_cost"])
    storage: RouterStorage = router
    for index in input_value["missing_ids"]:
        model_info = storage.model_list[index].get("model_info")
        if isinstance(model_info, dict):
            del model_info["id"]
    deployment_info = FixedDeploymentInfo(
        input_value["deployment_info"], input_value["lookup_error"]
    )
    setattr(router, DEPLOYMENT_INFO_METHOD, deployment_info)  # ast-grep-ignore: no-reflection
    parameters = FixedParameters(input_value["parameters"])
    setattr(litellm, SUPPORTED_PARAMS_FUNCTION, parameters)  # ast-grep-ignore: no-reflection
    provider_error = input_value["provider_error"]
    if provider_error != "none":
        failing = FailingProvider(provider_error == "bad_request")
        setattr(litellm, LLM_PROVIDER_FUNCTION, failing)  # ast-grep-ignore: no-reflection
    group: ModelGroupInfo | None = router.get_model_group_info(input_value["model_group"])
    if group is None:
        return None
    return ANSWER.validate_json(group.model_dump_json())
