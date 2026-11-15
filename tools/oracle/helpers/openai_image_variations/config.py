from collections.abc import Sequence
from typing import Protocol, TypedDict

import litellm
from litellm.llms.base_llm.image_variations.transformation import BaseImageVariationConfig
from litellm.types.utils import FileTypes, LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue as Json

litellm.suppress_debug_info = True


class RequestFields(TypedDict, total=False):
    data: dict[str, Json]


class VariationConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> Sequence[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        drop_params: bool,
    ) -> dict[str, Json]: ...

    def transform_request_image_variation(  # ast-grep-ignore
        self,
        model: str | None,
        image: FileTypes,
        optional_params: dict[str, Json],
        headers: dict[str, str],
    ) -> RequestFields: ...


def variation_config(model: str) -> VariationConfig:
    found: BaseImageVariationConfig | None = (
        ProviderConfigManager.get_provider_image_variation_config(
            model=model, provider=LlmProviders.OPENAI
        )
    )
    if found is None:
        raise ValueError(f"no image variation config is registered for {model}")
    return found
