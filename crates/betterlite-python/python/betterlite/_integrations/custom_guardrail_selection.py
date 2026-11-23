import json
from typing import Final

from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.custom_guardrail_constants import (
    NOT_PREMIUM_USER_MESSAGE,
    PRE_CALL_EXECUTED_GUARDRAILS_KEY,
    PRE_CALL_EXECUTED_TOKEN,
)
from betterlite._integrations.custom_guardrail_request import (
    admin_metadata,
    requested_list,
)
from betterlite._integrations.custom_guardrail_state import GuardrailState
from betterlite._integrations.custom_guardrail_support import (
    dynamic_params,
    event_hook_matches,
    hook_json,
    raise_native_error,
    selection_answer,
    settings_of,
)
from betterlite._integrations.custom_logger import LOGGER
from betterlite._native import NativeGuardrail, NativeGuardrailError
from betterlite.types.guardrails.params import GuardrailEventHooks, Mode

PRE_CALL_BUCKETS: Final = ("metadata", "litellm_metadata")


class GuardrailSelection(GuardrailState):
    @override
    def _validate_event_hook(
        self,
        event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None,
        supported_event_hooks: list[GuardrailEventHooks],
    ) -> None:
        settings = settings_of(self)
        settings["event_hook"] = hook_json(event_hook)
        settings["supported_event_hooks"] = [str(hook) for hook in supported_event_hooks]
        try:
            _ = NativeGuardrail(json.dumps(settings), strict_modes=True)
        except NativeGuardrailError as error:
            raise_native_error(error)

    @staticmethod
    def _get_admin_metadata(data: dict[str, JsonValue]) -> dict[str, JsonValue]:
        return admin_metadata(data)

    def get_disable_global_guardrail(self, data: dict[str, JsonValue]) -> bool | None:
        value = self._get_admin_metadata(data).get("disable_global_guardrails")
        match value:
            case bool():
                return value
            case None:
                return False
            case str() | int() | float() | list() | dict():
                return False

    # ast-grep-ignore: no-long-names
    def get_opted_out_global_guardrails_from_metadata(
        self, data: dict[str, JsonValue]
    ) -> list[str]:
        value = self._get_admin_metadata(data).get("opted_out_global_guardrails")
        names: list[str] = []
        match value:
            case list():
                for item in value:
                    match item:
                        case str():
                            names.append(item)
                        case bool() | int() | float() | list() | dict() | None:
                            pass
            case str() | bool() | int() | float() | dict() | None:
                pass
        return names

    def _is_valid_response_type(self, result: JsonValue) -> bool:
        match result:
            case None:
                return False
            case object():
                return True

    def get_guardrail_from_metadata(self, data: dict[str, JsonValue]) -> list[JsonValue]:
        if "guardrails" in data:
            return requested_list(data["guardrails"])
        for meta_key in PRE_CALL_BUCKETS:
            meta = data.get(meta_key)
            match meta:
                case dict():
                    if "guardrails" in meta:
                        return requested_list(meta["guardrails"])
                case str() | bool() | int() | float() | list() | None:
                    continue
        requested: list[JsonValue] = []
        return requested

    # ast-grep-ignore: no-long-names
    def _guardrail_is_in_requested_guardrails(
        self, requested_guardrails: list[JsonValue]
    ) -> bool:
        for requested in requested_guardrails:
            match requested:
                case dict():
                    if self.guardrail_name in requested:
                        return True
                case str():
                    if self.guardrail_name == requested:
                        return True
                case bool() | int() | float() | list() | None:
                    continue
        return False

    def _pre_call_marker(self) -> str | None:
        name = self.guardrail_name
        if not name:
            return None
        return f"{PRE_CALL_EXECUTED_TOKEN}:{name}"

    def mark_pre_call_hook_ran(self, data: dict[str, JsonValue]) -> None:
        marker = self._pre_call_marker()
        if marker is None:
            return
        for bucket_name in PRE_CALL_BUCKETS:
            bucket = data.get(bucket_name)
            match bucket:
                case dict():
                    executed = bucket.get(PRE_CALL_EXECUTED_GUARDRAILS_KEY)
                    match executed:
                        case list():
                            if marker not in executed:
                                executed.append(marker)
                        case str() | bool() | int() | float() | dict() | None:
                            fresh: list[JsonValue] = [marker]
                            bucket[PRE_CALL_EXECUTED_GUARDRAILS_KEY] = fresh
                    return
                case str() | bool() | int() | float() | list() | None:
                    pass
        entries: list[JsonValue] = [marker]
        fresh_bucket: dict[str, JsonValue] = {PRE_CALL_EXECUTED_GUARDRAILS_KEY: entries}
        data["metadata"] = fresh_bucket

    def _pre_call_hook_already_ran(self, data: dict[str, JsonValue]) -> bool:
        marker = self._pre_call_marker()
        if marker is None:
            return False
        for bucket_name in PRE_CALL_BUCKETS:
            bucket = data.get(bucket_name)
            match bucket:
                case dict():
                    executed = bucket.get(PRE_CALL_EXECUTED_GUARDRAILS_KEY)
                    match executed:
                        case list():
                            if marker in executed:
                                return True
                        case str() | bool() | int() | float() | dict() | None:
                            pass
                case str() | bool() | int() | float() | list() | None:
                    continue
        return False

    def _suppressed_by_auto_router_compression(self) -> bool:
        return False

    def should_run_guardrail(
        self,
        data: dict[str, JsonValue],
        event_type: GuardrailEventHooks,
        *,
        suppressed: frozenset[str] | None = None,
    ) -> bool:
        if self._suppressed_by_auto_router_compression():
            return False
        names = frozenset() if suppressed is None else suppressed
        return selection_answer(self._native, data, event_type, names)

    def _event_hook_is_event_type(self, event_type: GuardrailEventHooks) -> bool:
        return event_hook_matches(self._native, event_type)

    # ast-grep-ignore: no-long-names
    def get_guardrail_dynamic_request_body_params(
        self, request_data: dict[str, JsonValue]
    ) -> dict[str, JsonValue]:
        params = dynamic_params(self._native, request_data)
        if params:
            _ = self._validate_premium_user()
            LOGGER.warning(
                "Guardrail %s: ignoring dynamic extra_body %s without a LiteLLM enterprise license",
                self.guardrail_name,
                sorted(params),
            )
        return {}

    def _validate_premium_user(self) -> bool:
        LOGGER.warning(
            "Trying to use premium guardrail without premium user %s", NOT_PREMIUM_USER_MESSAGE
        )
        return False
