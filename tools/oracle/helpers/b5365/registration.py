import io
import logging
import struct
from collections.abc import Callable, Mapping, Sequence
from contextlib import redirect_stdout
from typing import Final, Protocol, TypeAlias, TypedDict, cast

import litellm
from litellm.utils import reapply_runtime_model_cost_registrations
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | Sequence["Json"] | Mapping[str, "Json"]
Entry: TypeAlias = dict[str, Json]
Entries: TypeAlias = dict[str, Entry]

WARNING_PREFIX: Final = "register_model: model="
WARNING_SUFFIX: Final = " has custom pricing"


class Registration(TypedDict):
    entries: Entries
    persist_across_reloads: bool
    warning_display_name: str | None


class Litellm(Protocol):
    model_cost: Entries
    open_ai_chat_completion_models: set[str]
    open_ai_text_completion_models: set[str]
    cohere_models: set[str]
    anthropic_models: set[str]
    openrouter_models: set[str]
    vercel_ai_gateway_models: set[str]
    edenai_models: set[str]
    vertex_text_models: set[str]
    vertex_code_text_models: set[str]
    vertex_chat_models: set[str]
    vertex_code_chat_models: set[str]
    ai21_models: set[str]
    nlp_cloud_models: set[str]
    aleph_alpha_models: set[str]
    bedrock_models: set[str]
    novita_models: set[str]

    def register_model(
        self,
        model_cost: Entries,
        *,
        persist_across_reloads: bool,
        warning_display_name: str | None,
    ) -> Entries: ...


class Utils(Protocol):
    _runtime_registered_model_cost: Entries

    def set_live_deployment_replay(self, replay: Callable[[], None]) -> None: ...


MODULE: Final[Litellm] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3317
        Litellm, litellm
    )
)
UTILS: Final[Utils] = (
    cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3136-L3479
        Utils, litellm.utils
    )
)
SETS: Final[dict[str, set[str]]] = {
    "open_ai_chat_completion_models": MODULE.open_ai_chat_completion_models,
    "open_ai_text_completion_models": MODULE.open_ai_text_completion_models,
    "cohere_models": MODULE.cohere_models,
    "anthropic_models": MODULE.anthropic_models,
    "openrouter_models": MODULE.openrouter_models,
    "vercel_ai_gateway_models": MODULE.vercel_ai_gateway_models,
    "edenai_models": MODULE.edenai_models,
    "vertex_text_models": MODULE.vertex_text_models,
    "vertex_code_text_models": MODULE.vertex_code_text_models,
    "vertex_chat_models": MODULE.vertex_chat_models,
    "vertex_code_chat_models": MODULE.vertex_code_chat_models,
    "ai21_models": MODULE.ai21_models,
    "nlp_cloud_models": MODULE.nlp_cloud_models,
    "aleph_alpha_models": MODULE.aleph_alpha_models,
    "bedrock_models": MODULE.bedrock_models,
    "novita_models": MODULE.novita_models,
}


class Scenario(TypedDict):
    model_cost: Entries
    registrations: list[Registration]
    live: list[Registration]
    refresh: Entries | None
    show: list[str]


def project(value: Json) -> Json:
    # numbers become text, since 3e-07 and 3e-7 are one float but two JSON spellings
    match value:
        case None | bool() | str():
            return value
        case int():
            return f"i:{value}"
        case float():
            return f"f:{struct.pack('>d', value).hex()}"
        case Sequence():
            return [project(item) for item in value]
        case Mapping():
            return {name: project(item) for name, item in value.items()}


def apply(registration: Registration) -> None:
    MODULE.register_model(
        registration["entries"],
        persist_across_reloads=registration["persist_across_reloads"],
        warning_display_name=registration["warning_display_name"],
    )


class LiveReplay:
    def __init__(self, registrations: list[Registration]) -> None:
        self.registrations = registrations

    def __call__(self) -> None:
        for registration in self.registrations:
            apply(registration)


class Recorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def register(scenario: Scenario) -> Json:
    MODULE.model_cost = scenario["model_cost"]
    UTILS._runtime_registered_model_cost.clear()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L3253
    for names in SETS.values():
        names.clear()
    UTILS.set_live_deployment_replay(LiveReplay(scenario["live"]))
    recorder = Recorder()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(recorder)
    # the provider lookup prints its provider list to stdout for a model with no provider
    with redirect_stdout(io.StringIO()):
        for registration in scenario["registrations"]:
            apply(registration)
        if scenario["refresh"] is not None:
            MODULE.model_cost = scenario["refresh"]
            reapply_runtime_model_cost_registrations()
    logger.removeHandler(recorder)
    warnings = [
        message.removeprefix(WARNING_PREFIX).partition(WARNING_SUFFIX)[0]
        for message in recorder.messages
        if message.startswith(WARNING_PREFIX)
    ]
    shown = {
        key: {
            field: value
            for field, value in entry.items()
            if field in scenario["show"] and value is not None
        }
        for key, entry in MODULE.model_cost.items()
    }
    sets = {name: sorted(names) for name, names in SETS.items() if names}
    return project({"model_cost": shown, "provider_sets": sets, "warnings": warnings})
