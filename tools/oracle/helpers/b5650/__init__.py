from collections.abc import Mapping
from importlib import import_module
from typing import Final, Literal, Protocol, TypedDict, cast

import litellm
from litellm.types.litellm_params import ControlOptions

from b5365.registration import Json, project
from b5650 import pricing


class Main(Protocol):
    def _resolve_control_options(
        self, kwargs: Mapping[str, Json], model: str
    ) -> ControlOptions: ...

    def _is_claude_tool_target(self, custom_llm_provider: str | None, model: str) -> bool: ...

    def _drop_anthropic_only_tool_keys(self, tools: list[Json] | None) -> list[Json] | None: ...


class Settings(Protocol):
    drop_params: bool


MAIN: Final[Main] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L347-L1225
        Main, import_module("litellm.main")
    )
)
SETTINGS: Final[Settings] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L249
        Settings, litellm
    )
)


class Control(TypedDict):
    function: Literal["control"]
    kwargs: dict[str, Json]
    model: str
    global_drop_params: bool | None


class ClaudeTarget(TypedDict):
    function: Literal["claude_target"]
    custom_llm_provider: str | None
    model: str


class ToolKeys(TypedDict):
    function: Literal["tool_keys"]
    tools: list[Json] | None


Input = Control | ClaudeTarget | ToolKeys | pricing.PricingInput


def run(call: Input) -> Json:
    if call["function"] == "control":
        SETTINGS.drop_params = call["global_drop_params"] is True
        resolved = MAIN._resolve_control_options(call["kwargs"], call["model"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L347
        return {"stream_chunk_size": resolved.stream_chunk_size}
    if call["function"] == "claude_target":
        return MAIN._is_claude_tool_target(call["custom_llm_provider"], call["model"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1200
    if call["function"] == "tool_keys":
        kept = MAIN._drop_anthropic_only_tool_keys(call["tools"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py#L1220
        return {"tools": project(kept)}
    return pricing.run(call)
