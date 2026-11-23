from typing import Final, Literal, NoReturn, Protocol, TypeAlias, runtime_checkable

from pydantic import JsonValue, TypeAdapter
from typing_extensions import TypedDict

from betterlite.types.guardrails.params import GuardrailEventHooks, Mode
from betterlite.types.utils.base import Dumpable

GuardrailStatus: TypeAlias = Literal[
    "success",
    "guardrail_flagged",
    "guardrail_intervened",
    "guardrail_failed_to_respond",
    "not_run",
]


class GenericGuardrailAPIInputs(TypedDict, total=False):
    texts: list[str]
    images: list[str]
    tools: list[dict[str, JsonValue]]
    tool_calls: list[dict[str, JsonValue]]
    structured_messages: list[dict[str, JsonValue]]
    model: str | None
    stream_holdback_chars: list[int]


# a guardrail receives and returns JSON, a LiteLLM object or the exception it caught
GuardrailValue: TypeAlias = (
    JsonValue | Dumpable | Exception | GenericGuardrailAPIInputs
    | list["GuardrailValue"] | dict[str, "GuardrailValue"]
)


class GuardrailTracingDetail(TypedDict, total=False):
    guardrail_id: str | None
    policy_template: str | None
    detection_method: str | None
    confidence_score: float | None
    classification: dict[str, JsonValue] | None
    match_details: list[dict[str, JsonValue]] | None
    patterns_checked: int | None
    alert_recipients: list[str] | None
    risk_score: float | None
    violation_categories: list[str] | None
    guardrail_action: str | None
    guardrail_usage: dict[str, int] | None
    guardrail_cost: float | None
    guardrail_cost_by_unit: dict[str, float | None] | None
    guardrail_cost_in_spend: bool | None


class SwitchesJson(TypedDict):
    use_native_during_call_hook: bool
    use_native_lifecycle_hooks: bool
    records_own_information: bool
    uses_apply_guardrail_interface: bool
    deny_text_on_error: bool


class SettingsJson(TypedDict):
    guardrail_name: str | None
    supported_event_hooks: list[str] | None
    event_hook: JsonValue
    default_on: bool
    mask_request_content: bool
    mask_response_content: bool
    violation_message_template: str | None
    end_session_after_n_fails: int | None
    on_violation: str | None
    realtime_violation_message: str | None
    on_sensitive_data: str | None
    sensitive_data_route_to_model: str | None
    sticky_session_routing: bool
    run_in_parallel: bool
    scan_raw_request: bool
    only_scan_new_messages: bool
    timeout: float | None
    switches: SwitchesJson


class RecordInput(TypedDict):
    guardrail_response: JsonValue
    guardrail_status: GuardrailStatus
    start_time: float | None
    end_time: float | None
    duration: float | None
    masked_entity_count: dict[str, int] | None
    guardrail_provider: str | None
    event_type: str | None
    tracing_detail: JsonValue | None


class LogTarget(TypedDict):
    litellm_params: dict[str, JsonValue] | None
    model_call_params: dict[str, JsonValue] | None


class RecordContext(TypedDict):
    settings: SettingsJson
    log_target: LogTarget | None


class GuardrailConfigModel(Protocol): ...


class GuardrailRecorder(Protocol):
    # ast-grep-ignore: no-long-names
    def add_standard_logging_guardrail_information_to_request_data(
        self, guardrail_json_response: GuardrailValue, request_data: dict[str, JsonValue],
        guardrail_status: GuardrailStatus, start_time: float | None = None,
        end_time: float | None = None,
    ) -> None: ...

    def _process_response(
        self, response: GuardrailValue, request_data: dict[str, JsonValue],
        start_time: float | None = None, end_time: float | None = None,
    ) -> GuardrailValue: ...

    def _process_error(
        self, e: Exception, request_data: dict[str, JsonValue],
        start_time: float | None = None, end_time: float | None = None,
    ) -> NoReturn: ...


@runtime_checkable
class GuardrailCache(Protocol):
    async def async_get_cache(self, key: str) -> JsonValue: ...

    async def async_set_cache(self, key: str, value: list[str], ttl: int) -> None: ...


class GuardrailSettingsOwner(Protocol):
    guardrail_name: str | None
    supported_event_hooks: list[GuardrailEventHooks] | None
    event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None
    default_on: bool
    mask_request_content: bool
    mask_response_content: bool
    violation_message_template: str | None
    end_session_after_n_fails: int | None
    on_violation: str | None
    realtime_violation_message: str | None
    on_sensitive_data: str | None
    sensitive_data_route_to_model: str | None
    sticky_session_routing: bool
    run_in_parallel: bool
    scan_raw_request: bool
    only_scan_new_messages: bool
    timeout: float | None
    use_native_during_call_hook: bool
    use_native_lifecycle_hooks: bool
    # ast-grep-ignore: no-long-names
    records_own_guardrail_information: bool

    def uses_apply_guardrail_interface(self) -> bool: ...


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
SETTINGS: Final[TypeAdapter[SettingsJson]] = TypeAdapter(SettingsJson)
LOG_TARGET: Final[TypeAdapter[LogTarget | None]] = TypeAdapter(LogTarget | None)
