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
    as_list,
    as_mapping,
    dynamic_params,
    event_hook_matches,
    native_handle,
    selection_answer,
)
from betterlite._integrations.custom_logger import LOGGER
from betterlite.types.guardrails.params import GuardrailEventHooks, Mode

PRE_CALL_BUCKETS: Final = ("metadata", "litellm_metadata")


class GuardrailSelection(GuardrailState):
    @override
    def _validate_event_hook(
        self,
        event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None,
        supported_event_hooks: list[GuardrailEventHooks],
    ) -> None:
        _ = (event_hook, supported_event_hooks)
        _ = native_handle(self, strict=True)

    @staticmethod
    def _get_admin_metadata(data: dict[str, JsonValue]) -> dict[str, JsonValue]:
        return admin_metadata(data)

    def get_disable_global_guardrail(self, data: dict[str, JsonValue]) -> bool | None:
        value = self._get_admin_metadata(data).get("disable_global_guardrails")
        if value is None:
            return False
        match value:
            case bool():
                return value
            case str() | int() | float() | list() | dict():
                return False

    # ast-grep-ignore: no-long-names
    def get_opted_out_global_guardrails_from_metadata(
        self, data: dict[str, JsonValue]
    ) -> list[str]:
        value = self._get_admin_metadata(data).get("opted_out_global_guardrails")
        listed = as_list(value)
        names: list[str] = []
        if listed is None:
            return names
        for item in listed:
            match item:
                case str():
                    names.append(item)
                case bool() | int() | float() | list() | dict() | None:
                    pass
        return names

    def _is_valid_response_type(self, result: JsonValue) -> bool:
        return result is not None

    def get_guardrail_from_metadata(self, data: dict[str, JsonValue]) -> list[JsonValue]:
        if "guardrails" in data:
            return requested_list(data["guardrails"])
        for meta_key in PRE_CALL_BUCKETS:
            meta = as_mapping(data.get(meta_key))
            if meta is not None and "guardrails" in meta:
                return requested_list(meta["guardrails"])
        return []

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
        # ast-grep-ignore: no-long-names
        bucket_key = PRE_CALL_EXECUTED_GUARDRAILS_KEY
        for bucket_name in PRE_CALL_BUCKETS:
            bucket = as_mapping(data.get(bucket_name))
            if bucket is None:
                continue
            executed = as_list(bucket.get(bucket_key))
            if executed is None:
                fresh: list[JsonValue] = [marker]
                bucket[bucket_key] = fresh
            elif marker not in executed:
                executed.append(marker)
            return
        entries: list[JsonValue] = [marker]
        fresh_bucket: dict[str, JsonValue] = {bucket_key: entries}
        data["metadata"] = fresh_bucket

    def _pre_call_hook_already_ran(self, data: dict[str, JsonValue]) -> bool:
        marker = self._pre_call_marker()
        if marker is None:
            return False
        # ast-grep-ignore: no-long-names
        bucket_key = PRE_CALL_EXECUTED_GUARDRAILS_KEY
        for bucket_name in PRE_CALL_BUCKETS:
            bucket = as_mapping(data.get(bucket_name))
            if bucket is None:
                continue
            executed = as_list(bucket.get(bucket_key))
            if executed is not None and marker in executed:
                return True
        return False

    # ast-grep-ignore: no-long-names
    def _suppressed_by_auto_router_compression(self) -> bool:
        return False

    def should_run_guardrail(
        self, data: dict[str, JsonValue], event_type: GuardrailEventHooks,
        *, suppressed: frozenset[str] | None = None,
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
