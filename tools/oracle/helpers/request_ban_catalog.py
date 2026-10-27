from typing import Literal, TypedDict

from litellm.proxy.auth.auth_utils import (
    _BANNED_REQUEST_BODY_PARAMS,  # pyright: ignore[reportPrivateUsage]
    _NESTED_CONFIG_KEYS,  # pyright: ignore[reportPrivateUsage]
    _NESTED_METADATA_KEYS,  # pyright: ignore[reportPrivateUsage]
    _build_banned_observability_params,  # pyright: ignore[reportPrivateUsage]
)


class Input(TypedDict):
    function: Literal["catalog", "observability"]


class Catalog(TypedDict):
    banned: list[str]
    nested_config: list[str]
    metadata: list[str]


def run(input_value: Input) -> Catalog | list[str]:
    match input_value["function"]:
        case "catalog":
            return {
                "banned": list(_BANNED_REQUEST_BODY_PARAMS),
                "nested_config": list(_NESTED_CONFIG_KEYS),
                "metadata": list(_NESTED_METADATA_KEYS),
            }
        case "observability":
            return sorted(_build_banned_observability_params())
