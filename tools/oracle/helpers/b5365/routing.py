from collections.abc import Callable
from typing import Final, NotRequired, Protocol, TypedDict, Unpack, cast

import litellm
import litellm.types.router

from b5365.registration import Json


class Params(TypedDict):
    model: str
    custom_llm_provider: NotRequired[str]
    api_base: NotRequired[str]
    region_name: NotRequired[str]
    vertex_location: NotRequired[str]
    aws_region_name: NotRequired[str]
    watsonx_region_name: NotRequired[str]


class Order(TypedDict):
    order: NotRequired[int | None]


class Deployment(TypedDict):
    id: str
    litellm_params: NotRequired[Order]
    model_info: NotRequired[Order]


class RegionConfig(Protocol):
    def get_eu_regions(self) -> list[str]: ...

    def get_us_regions(self) -> list[str]: ...


class ParamsModel(Protocol): ...


class ParamsFactory(Protocol):
    def __call__(self, **fields: Unpack[Params]) -> ParamsModel: ...


class RouterTypes(Protocol):
    LiteLLM_Params: ParamsFactory


class Litellm(Protocol):
    vertex_location: str | None
    AzureOpenAIConfig: Callable[[], RegionConfig]
    VertexAIConfig: Callable[[], RegionConfig]
    AmazonBedrockGlobalConfig: Callable[[], RegionConfig]
    IBMWatsonXAIConfig: Callable[[], RegionConfig]


class Utils(Protocol):
    def _get_deployment_order(self, deployment: Deployment) -> int | None: ...

    def get_order_filtered_deployments(
        self, healthy_deployments: list[Deployment], target_order: int | None = None
    ) -> list[Deployment]: ...

    def _get_model_region(
        self, custom_llm_provider: str, litellm_params: ParamsModel
    ) -> str | None: ...

    def _infer_model_region(self, litellm_params: ParamsModel) -> str | None: ...

    def is_region_allowed(self, litellm_params: ParamsModel, allowed_model_region: str) -> bool: ...


MODULE: Final[Litellm] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5270-L5328
        Litellm, litellm
    )
)
ROUTER: Final[RouterTypes] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/router.py#L522
        RouterTypes, litellm.types.router
    )
)
UTILS: Final[Utils] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5188-L5352
        Utils, litellm.utils
    )
)


def order_filter(deployments: list[Deployment], target_order: int | None) -> Json:
    kept = UTILS.get_order_filtered_deployments(deployments, target_order)
    orders = [UTILS._get_deployment_order(deployment) for deployment in deployments]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5188
    return {"orders": orders, "kept": [deployment["id"] for deployment in kept]}


def region_lists() -> dict[str, Json]:
    configs = {
        "azure": MODULE.AzureOpenAIConfig(),
        "vertex_ai": MODULE.VertexAIConfig(),
        "bedrock": MODULE.AmazonBedrockGlobalConfig(),
        "watsonx": MODULE.IBMWatsonXAIConfig(),
    }
    return {
        provider: {"eu": config.get_eu_regions(), "us": config.get_us_regions()}
        for provider, config in configs.items()
    }


def region(
    params: Params,
    default_vertex_location: str | None,
    provider: str | None,
    allowed_model_region: str,
) -> Json:
    MODULE.vertex_location = default_vertex_location
    litellm_params = ROUTER.LiteLLM_Params(**params)
    inferred = UTILS._infer_model_region(litellm_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5270
    model_region = (
        None if provider is None else UTILS._get_model_region(provider, litellm_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L5245
    )
    return {
        "lists": region_lists(),
        "model_region": model_region,
        "inferred": inferred,
        "allowed": UTILS.is_region_allowed(litellm_params, allowed_model_region),
    }
