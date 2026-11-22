from typing import TypedDict

from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager, supports_httpx_timeout


class Input(TypedDict):
    model: str
    config_providers: list[str]
    timeout_providers: list[str | None]


class Output(TypedDict):
    configs: list[str | None]
    capabilities: list[bool]


def config_class(provider: str, model: str) -> str | None:
    config = ProviderConfigManager.get_provider_files_config(
        model=model, provider=LlmProviders(provider)
    )
    return None if config is None else type(config).__name__


def timeout_capability(provider: str | None) -> bool:
    if provider is None:
        return False
    return supports_httpx_timeout(provider)


def run(input_value: Input) -> Output:
    return Output(
        configs=[
            config_class(provider, input_value["model"])
            for provider in input_value["config_providers"]
        ],
        capabilities=[
            timeout_capability(provider) for provider in input_value["timeout_providers"]
        ],
    )
