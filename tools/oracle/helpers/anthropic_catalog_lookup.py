from typing import NotRequired, TypedDict
from unittest.mock import patch

import litellm
import litellm.utils
from litellm.llms.anthropic.common_utils import AnthropicModelInfo

Catalog = dict[str, object]


class Input(TypedDict):
    function: str
    model: str
    key: NotRequired[str]
    provider: NotRequired[str]
    runtime: NotRequired[Catalog]
    packaged: NotRequired[Catalog]


def run(data: Input) -> object:
    model = data["model"]
    function = data["function"]
    if function == "candidates":
        return AnthropicModelInfo._model_map_lookup_candidates(model)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    bundled = data.get("packaged", {})
    with (
        patch.object(litellm, "model_cost", data.get("runtime", {})),
        patch.object(litellm.utils, "_get_bundled_model_cost_map", return_value=bundled),  # pyright: ignore[reportPrivateUsage]
    ):
        key = data.get("key", "")
        if function == "capability":
            return AnthropicModelInfo._get_model_capability(model, key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        if function == "exact":
            return AnthropicModelInfo._get_exact_model_capability(model, key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return AnthropicModelInfo.supports_fast_mode(model, data.get("provider", ""))
