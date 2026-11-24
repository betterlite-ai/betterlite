from pydantic import JsonValue

from betterlite._integrations.custom_guardrail_hooks import GuardrailHooks
from betterlite._integrations.custom_guardrail_support import (
    call_type_messages,
    native_settings,
    update_native_params,
)
from betterlite._integrations.custom_logger_types import CallTypes
from betterlite.types.guardrails.params import LitellmParams
from betterlite.types.utils.base import InitValue


class GuardrailParams(GuardrailHooks):
    # ast-grep-ignore: no-long-names
    def update_in_memory_litellm_params(self, litellm_params: LitellmParams) -> None:
        payload: dict[str, JsonValue] = litellm_params.model_dump(mode="json", exclude_none=True)
        update_native_params(self._native(), payload)
        settings = native_settings(self._native())
        self.default_on = settings["default_on"]
        self.mask_request_content = settings["mask_request_content"]
        self.mask_response_content = settings["mask_response_content"]
        self.violation_message_template = settings["violation_message_template"]
        self.end_session_after_n_fails = settings["end_session_after_n_fails"]
        self.on_violation = settings["on_violation"]
        self.realtime_violation_message = settings["realtime_violation_message"]
        self.on_sensitive_data = settings["on_sensitive_data"]
        self.sensitive_data_route_to_model = settings["sensitive_data_route_to_model"]
        self.sticky_session_routing = settings["sticky_session_routing"]
        self.run_in_parallel = settings["run_in_parallel"]
        self.scan_raw_request = settings["scan_raw_request"]
        self.only_scan_new_messages = settings["only_scan_new_messages"]
        self.timeout = settings["timeout"]
        for name in LitellmParams.model_fields:
            # ast-grep-ignore: no-reflection
            field_value: InitValue = getattr(litellm_params, name)
            # ast-grep-ignore: no-reflection
            setattr(self, name, field_value)

    def get_guardrails_messages_for_call_type(  # ast-grep-ignore: no-long-names
        self, call_type: CallTypes | None, data: dict[str, JsonValue] | None = None
    ) -> list[JsonValue] | None:
        if call_type is None or data is None:
            return None
        selected = call_type_messages(call_type, data)
        match selected:
            case list():
                return selected
            case str() | bool() | int() | float() | dict() | None:
                return None
