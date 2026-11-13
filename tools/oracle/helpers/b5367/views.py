import datetime as dt
from typing import Final, Literal, Protocol, Required, TypeAlias, TypedDict, Unpack, cast

import litellm
from litellm import utils as litellm_utils
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils import litellm_logging
from litellm.litellm_core_utils.rules import Rules
from litellm.models.credentials import CredentialItem
from typing_extensions import override

from b5367.markers import CallbackValue, Entry, Json, Markers


class ListsSpec(TypedDict, total=False):
    input: list[Entry]
    success: list[Entry]
    async_success: list[Entry]
    failure: list[Entry]
    async_failure: list[Entry]


class Outcome(TypedDict, total=False):
    label: str
    error: bool


class DecisionSpec(TypedDict, total=False):
    decision: bool | None
    message: str | None


Decision: TypeAlias = bool | DecisionSpec


class CredentialSpec(TypedDict):
    credential_name: str
    credential_values: dict[str, str]


class Input(TypedDict, total=False):
    action: Required[Literal["setup", "credentials", "request_flags"]]
    request: Required[dict[str, Json]]
    call_type: Required[str]
    is_async_call: bool
    is_pass_through: bool
    lists: ListsSpec
    callbacks: list[Entry]
    resolver: dict[str, Outcome]
    rules: list[Decision | None]
    provider_map: list[str]
    providers: list[str]
    custom_providers: list[str]
    credentials: list[CredentialSpec]


# LiteLLM passes these keys by name, and four of them pass the name length limit
LoggingKwargs = TypedDict(  # noqa: UP013  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L1294-L1310
    "LoggingKwargs",
    {
        "model": str,
        "messages": Json,
        "stream": bool,
        "litellm_call_id": str,
        "litellm_trace_id": str | None,
        "function_id": str,
        "call_type": str,
        "start_time": dt.datetime,
        "dynamic_success_callbacks": list[CallbackValue] | None,
        "dynamic_failure_callbacks": list[CallbackValue] | None,
        "dynamic_async_success_callbacks": list[CallbackValue] | None,
        "dynamic_async_failure_callbacks": list[CallbackValue] | None,
        "kwargs": dict[str, Json],
        "applied_guardrails": list[str] | None,
        "supports_correlation_logging": bool,
    },
)


class FixedRule:
    def __init__(self, decision: Decision) -> None:
        self.decision = decision

    def __call__(self, _input: str) -> Decision:
        return self.decision


class Recording:
    def __init__(self) -> None:
        self.updates: list[dict[str, Json]] = []


class StubLogging(Recording):
    def __init__(self, **fields: Unpack[LoggingKwargs]) -> None:
        super().__init__()
        self.fields = fields

    def update_environment_variables(self, **kwargs: Json) -> None:
        self.updates.append(kwargs)


class LoggingClassGetter:
    def __call__(self) -> type[StubLogging]:
        return StubLogging


class Recorder:
    def __init__(self, outcomes: dict[str, Outcome] | None, markers: Markers) -> None:
        self.outcomes = outcomes
        self.markers = markers

    def __call__(
        self, logging_integration: str, internal_usage_cache: None, llm_router: None
    ) -> CustomLogger | None:
        del internal_usage_cache, llm_router
        if self.outcomes is None:
            return None
        outcome = self.outcomes.get(logging_integration)
        if outcome is None or outcome.get("error") is True:
            return None
        label = outcome.get("label")
        if label is None:
            return None
        return self.markers.marker(label, None)


class LiteLLMState(Protocol):
    callbacks: list[CallbackValue]
    input_callback: list[CallbackValue]
    success_callback: list[CallbackValue]
    failure_callback: list[CallbackValue]
    _async_success_callback: list[CallbackValue]
    _async_failure_callback: list[CallbackValue]
    custom_provider_map: list[dict[str, str]]
    provider_list: list[str]
    _custom_providers: list[str]
    pre_call_rules: list[FixedRule | None]
    credential_list: list[CredentialItem]


class UtilsView(Protocol):
    callback_list: list[str]
    get_litellm_logging_class: LoggingClassGetter

    def function_setup(
        self,
        original_function: str,
        rules_obj: Rules,
        start_time: dt.datetime,
        *,
        is_async_call: bool,
        **kwargs: Json,
    ) -> tuple[StubLogging, dict[str, Json]]: ...

    def load_credentials_from_list(self, kwargs: dict[str, Json]) -> None: ...

    def _is_async_request(self, kwargs: dict[str, Json], *, is_pass_through: bool) -> bool: ...

    def _is_streaming_request(self, kwargs: dict[str, Json], call_type: str) -> bool: ...


class LoggingView(Protocol):
    @override
    def __setattr__(self, name: str, value: Recorder, /) -> None: ...


STATE: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L115-L206
    LiteLLMState, litellm
)
UTILS: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L981-L988
    UtilsView, litellm_utils
)
LOGGING: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L4502-L4507
    LoggingView, litellm_logging
)
