from collections.abc import Callable
from typing import Protocol, TypeAlias, TypedDict

from litellm.llms.base_llm.image_variations.transformation import BaseImageVariationConfig
from litellm.types.llms.openai import OpenAIImageVariationOptionalParams
from litellm.types.utils import FileTypes, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue as Json


class RequestFields(TypedDict, total=False):
    data: dict[str, Json]


Supported: TypeAlias = Callable[[str], list[OpenAIImageVariationOptionalParams]]  # ast-grep-ignore
Transform: TypeAlias = Callable[
    [str | None, FileTypes, dict[str, Json], dict[str, str]], RequestFields
]


class Mapper(Protocol):
    def __call__(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        *,
        drop_params: bool,
    ) -> dict[str, Json]: ...


class VariationConfig(Protocol):
    @property
    def get_supported_openai_params(self) -> Supported: ...

    @property
    def map_openai_params(self) -> Mapper: ...

    @property
    def transform_request_image_variation(self) -> Transform: ...  # ast-grep-ignore


def variation_config(model: str) -> VariationConfig:
    found = ProviderConfigManager.get_provider_image_variation_config(
        model=model, provider=LlmProviders.OPENAI
    )
    if found is None:
        raise ValueError(f"no image variation config is registered for {model}")
    return found
