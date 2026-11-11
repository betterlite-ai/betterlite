from typing import TYPE_CHECKING, TypeAlias, cast

from litellm.litellm_core_utils.get_provider_specific_headers import (
    ProviderSpecificHeaderUtils,
)

if TYPE_CHECKING:
    from litellm.types.utils import ProviderSpecificHeader

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


def object_value(value: Json, name: str) -> Object:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    return value


def object_list(value: Json, name: str) -> list[Object]:
    if not isinstance(value, list):
        raise TypeError(f"{name} must be a list")
    return [object_value(entry, name) for entry in value]


def run(input_value: Object) -> Json:
    entries = object_list(input_value.get("entries", []), "entries")
    provider = input_value.get("custom_llm_provider")
    if provider is not None and not isinstance(provider, str):
        raise TypeError("custom_llm_provider must be a string")
    return cast(
        Json,
        ProviderSpecificHeaderUtils.get_provider_specific_headers(  # pyright: ignore[reportUnknownMemberType]
            cast("list[ProviderSpecificHeader]", entries),
            provider,
        ),
    )
