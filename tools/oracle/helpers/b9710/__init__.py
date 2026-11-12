import copy
from collections.abc import Mapping, Sequence
from typing import Final, NotRequired, TypeAlias, TypedDict, cast

import litellm
from litellm.router import Router
from litellm.types.router import Deployment, LiteLLM_Params, ModelInfo
from litellm.utils import _invalidate_model_cost_lowercase_map  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5576

Json: TypeAlias = None | bool | int | float | str | Sequence["Json"] | Mapping[str, "Json"]
Entry: TypeAlias = dict[str, Json]
Entries: TypeAlias = dict[str, Entry]


class Params(TypedDict):
    model: str
    custom_llm_provider: NotRequired[str]


class Info(TypedDict):
    id: str


class DeploymentInput(TypedDict):
    model_name: str
    litellm_params: Params
    model_info: Info


class Input(TypedDict):
    deployment: DeploymentInput
    shared_before: Entry | None


class Answer(TypedDict):
    by_id: Entry | None
    shared: Entries


def run(input_value: Input) -> Answer:
    given: Final = input_value["deployment"]
    deployment: Final = Deployment(
        model_name=given["model_name"],
        litellm_params=LiteLLM_Params(**given["litellm_params"]),
        model_info=ModelInfo(**given["model_info"]),
    )
    backend_keys: Final = Router._backend_cost_map_keys(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L10024
        deployment.litellm_params.model, deployment.litellm_params.custom_llm_provider
    )
    model_cost: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
        Entries, litellm.model_cost
    )
    saved: Final = copy.deepcopy(model_cost)
    try:
        if input_value["shared_before"] is not None:
            model_cost[backend_keys[0]] = input_value["shared_before"]
        Router._register_deployment_pricing(deployment)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L10069
        model_id: Final = given["model_info"]["id"]
        # the lookup adds supported_openai_params and null fields that depend on the provider
        shown: Final[Entries] = {
            key: {
                field: value
                for field, value in model_cost[key].items()
                if value is not None and field != "supported_openai_params"
            }
            for key in (model_id, *backend_keys)
            if key in model_cost
        }
        shared: Final = {key: shown[key] for key in backend_keys if key in shown}
        return {"by_id": shown.get(model_id), "shared": shared}
    finally:
        model_cost.clear()
        model_cost.update(saved)
        _invalidate_model_cost_lowercase_map()
