import json
from collections.abc import Mapping
from typing import TypeAlias, cast

from litellm.llms.openai_like.json_loader import JSONProviderRegistry, SimpleProviderConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    source = input_value.get("source")
    if isinstance(source, str):
        raw = cast(dict[str, Json], json.loads(source))
        descriptors: list[Json] = []
        for slug, config_value in raw.items():
            if not isinstance(config_value, dict):
                break
            config = cast(dict[str, Json], config_value)
            if not isinstance(config.get("base_url"), str) or not isinstance(
                config.get("api_key_env"), str
            ):
                break
            descriptors.append(_descriptor(slug, config))
    else:
        saved_providers = JSONProviderRegistry._providers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        saved_loaded = JSONProviderRegistry._loaded  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        try:
            JSONProviderRegistry._providers = {}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            JSONProviderRegistry._loaded = False  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            JSONProviderRegistry.load()
            descriptors = [
                _simple_descriptor(slug, provider)
                for slug, provider in JSONProviderRegistry._providers.items()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            ]
        finally:
            JSONProviderRegistry._providers = saved_providers  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            JSONProviderRegistry._loaded = saved_loaded  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    result: dict[str, Json] = {"entries": descriptors, "count": len(descriptors)}
    return result


def _descriptor(slug: str, config: Mapping[str, Json]) -> dict[str, Json]:
    if not isinstance(config.get("base_url"), str) or not isinstance(
        config.get("api_key_env"), str
    ):
        raise ValueError("provider requires base_url and api_key_env")
    return {
        "slug": slug,
        "base_url": config["base_url"],
        "api_key_env": config["api_key_env"],
        "api_base_env": config.get("api_base_env"),
        "base_class": config.get("base_class", "openai_gpt"),
        "param_mappings": config.get("param_mappings", {}),
        "constraints": config.get("constraints", {}),
        "special_handling": config.get("special_handling", {}),
        "supported_endpoints": config.get("supported_endpoints", []),
    }


def _simple_descriptor(slug: str, provider: SimpleProviderConfig) -> dict[str, Json]:
    return _descriptor(
        slug,
        cast(Mapping[str, Json], {
            "base_url": provider.base_url,
            "api_key_env": provider.api_key_env,
            "api_base_env": provider.api_base_env,
            "base_class": provider.base_class,
            "param_mappings": provider.param_mappings,
            "constraints": provider.constraints,
            "special_handling": provider.special_handling,
            "supported_endpoints": provider.supported_endpoints,
        }),
    )
