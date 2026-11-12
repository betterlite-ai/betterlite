import logging
from typing import Literal, TypedDict, cast

import litellm
from litellm.anthropic_beta_headers_manager import (
    _load_beta_headers_config,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L162
    reload_beta_headers_config,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L186
)
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# the start of each LiteLLM message and the diagnostic kind betterlite reports for it
KINDS: tuple[tuple[str, str], ...] = (
    ("Failed to load local beta headers config", "local_load_failed"),
    ("LiteLLM: Fetched beta headers config is not a dict", "not_a_dict"),
    ("LiteLLM: Fetched beta headers config is empty", "empty"),
    (
        "LiteLLM: Fetched beta headers config missing provider keys",
        "missing_provider_keys",
    ),
    ("LiteLLM: Failed to fetch remote beta headers config", "fetch_failed"),
    (
        "LiteLLM: Fetched beta headers config failed integrity check",
        "integrity_failed",
    ),
    ("Loaded and cached beta headers config", "cache_filled"),
    ("Reloading beta headers config", "reloading"),
)


class BetaConfigCall(TypedDict):
    url: Literal["mock", "empty"]
    steps: list[Literal["load", "reload"]]


class Diagnostic(TypedDict):
    level: str
    kind: str


class Step(TypedDict):
    matrix: dict[str, JsonValue]
    diagnostics: list[Diagnostic]


class BetaConfigRun(TypedDict):
    steps: list[Step]


class BetaConfigValue(TypedDict):
    value: BetaConfigRun


class BetaConfigOutcome(TypedDict):
    beta_config: BetaConfigValue


class Recorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.diagnostics: list[Diagnostic] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        for prefix, kind in KINDS:
            if message.startswith(prefix):
                self.diagnostics.append({"level": record.levelname.lower(), "kind": kind})


def run_beta_config(call: BetaConfigCall, api_base: str) -> str:
    urls: dict[Literal["mock", "empty"], str] = {
        "mock": f"{api_base}/anthropic_beta_headers_config.json",
        "empty": "",
    }
    litellm.anthropic_beta_headers_url = urls[call["url"]]
    recorder = Recorder()
    logger = logging.getLogger("LiteLLM")
    previous_level = logger.level
    logger.addHandler(recorder)
    logger.setLevel(logging.DEBUG)
    steps: list[Step] = []
    for step in call["steps"]:
        if step == "reload":
            matrix = cast("dict[str, JsonValue]", reload_beta_headers_config())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L186
        else:
            matrix = cast("dict[str, JsonValue]", _load_beta_headers_config())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L162
        steps.append({"matrix": matrix, "diagnostics": recorder.diagnostics})
        recorder.diagnostics = []
    # LiteLLM's exit hook logs at DEBUG to stdout once the redirect ends and would trail the JSON
    logger.removeHandler(recorder)
    logger.setLevel(previous_level)
    outcome: BetaConfigOutcome = {"beta_config": {"value": {"steps": steps}}}
    return TypeAdapter(BetaConfigOutcome).dump_json(outcome).decode()
