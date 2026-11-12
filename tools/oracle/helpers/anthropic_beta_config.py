from typing import Literal, TypeAlias, TypedDict, cast

from litellm import anthropic_beta_headers_manager
from litellm.anthropic_beta_headers_manager import (
    GetAnthropicBetaHeadersConfig,
    get_provider_name,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    function: Literal["validate", "provider_name"]
    matrix: Json
    provider: str


def run(input_value: Input) -> Json:
    matrix = cast("dict[str, Json]", input_value["matrix"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L101
    if input_value["function"] == "validate":
        valid = GetAnthropicBetaHeadersConfig.validate_beta_headers_config(matrix)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L101
        return {"valid": valid}
    anthropic_beta_headers_manager._BETA_HEADERS_CONFIG = matrix  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L35
    return get_provider_name(input_value["provider"])
