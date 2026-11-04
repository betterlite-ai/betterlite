from collections.abc import Mapping
from io import StringIO
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.llms.openai_like import json_loader
from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    source = input_value.get("source")
    missing = input_value.get("missing") is True
    saved_providers = JSONProviderRegistry._providers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    saved_loaded = JSONProviderRegistry._loaded  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        JSONProviderRegistry._providers = {}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        JSONProviderRegistry._loaded = False  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        if isinstance(source, str) or missing:
            _load_fixture(source if isinstance(source, str) else None)
        else:
            JSONProviderRegistry.load()
        descriptors: list[Json] = [
            _simple_descriptor(slug, provider)
            for slug, provider in JSONProviderRegistry._providers.items()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        ]
    finally:
        JSONProviderRegistry._providers = saved_providers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        JSONProviderRegistry._loaded = saved_loaded  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    result: dict[str, Json] = {"entries": descriptors, "count": len(descriptors)}
    return result


def _load_fixture(source: str | None) -> None:
    class FixturePath:
        def __init__(self, path: str) -> None:
            self.path = path

        @property
        def parent(self) -> "FixturePath":
            return self

        def __truediv__(self, _: str) -> "FixturePath":
            return self

        def exists(self) -> bool:
            return source is not None

    def read_fixture(_: object) -> StringIO:
        if source is None:
            raise FileNotFoundError
        return StringIO(source)

    with patch.object(json_loader, "Path", FixturePath), patch("builtins.open", read_fixture):
        JSONProviderRegistry.load()


def _simple_descriptor(slug: str, provider: SimpleProviderConfig) -> dict[str, Json]:
    values: Mapping[str, Json] = cast(
        Mapping[str, Json],
        {
            "base_url": provider.base_url,
            "api_key_env": provider.api_key_env,
            "api_base_env": provider.api_base_env,
            "base_class": provider.base_class,
            "param_mappings": provider.param_mappings,
            "constraints": provider.constraints,
            "special_handling": provider.special_handling,
            "supported_endpoints": provider.supported_endpoints,
        },
    )
    return {
        "slug": slug,
        "base_url": values["base_url"],
        "api_key_env": values["api_key_env"],
        "api_base_env": values["api_base_env"],
        "base_class": values["base_class"],
        "param_mappings": values["param_mappings"],
        "constraints": values["constraints"],
        "special_handling": values["special_handling"],
        "supported_endpoints": values["supported_endpoints"],
    }
