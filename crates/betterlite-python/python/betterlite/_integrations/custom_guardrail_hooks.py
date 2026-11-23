from collections.abc import Awaitable, Callable, Mapping, Sequence
from typing import Final, Literal, TypeAlias

from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.custom_guardrail_recording import GuardrailRecording
from betterlite._integrations.custom_logger import LOGGER, CustomLogger
from betterlite._integrations.custom_logger_types import (
    CallTypes,
    LLMResponseTypes,
    LiteLLMLoggingObj,
)
from betterlite._integrations.guardrail_types import GenericGuardrailAPIInputs
from betterlite.types.guardrails.params import GuardrailEventHooks
from betterlite.types.litellm_params_state import JsonMap

PRE_CALL_CALL_TYPES: Final[tuple[str, ...]] = ("completion", "acompletion")

ApplyGuardrailCall: TypeAlias = Callable[..., Awaitable[GenericGuardrailAPIInputs]]


class GuardrailHooks(GuardrailRecording):
    async def apply_guardrail(
        self,
        inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
        logging_obj: LiteLLMLoggingObj | None = None,
    ) -> GenericGuardrailAPIInputs:
        _ = (request_data, input_type, logging_obj)
        return inputs

    def uses_apply_guardrail_interface(self) -> bool:
        own: ApplyGuardrailCall = self.__class__.apply_guardrail
        base: ApplyGuardrailCall = GuardrailHooks.apply_guardrail
        return own is not base

    def _deployment_hook_target(self) -> CustomLogger:
        if not self.uses_apply_guardrail_interface() or self.use_native_lifecycle_hooks:
            return self
        name = self.guardrail_name or self.__class__.__name__
        message = (
            f"Guardrail {name} implements apply_guardrail, which needs the litellm proxy "
            "dependencies to run at the deployment level. Install them with: "
            "pip install 'litellm[proxy]'"
        )
        raise ImportError(message)

    @override
    async def async_pre_call_deployment_hook(
        self, kwargs: JsonMap, call_type: CallTypes | None
    ) -> dict[str, JsonValue] | None:
        data = dict(kwargs)
        guardrails = data.get("guardrails")
        match guardrails:
            case list():
                pass
            case str() | bool() | int() | float() | dict() | None:
                return data
        if self._pre_call_hook_already_ran(data):
            return data
        if not self.should_run_guardrail(data, GuardrailEventHooks.pre_call):
            return data
        if call_type in PRE_CALL_CALL_TYPES:
            _ = self._deployment_hook_target()
        return data

    async def async_pre_call_hook_on_messages(  # ast-grep-ignore: no-long-names
        self,
        request_data: Mapping[str, JsonValue],
        messages: Sequence[JsonValue],
    ) -> tuple[JsonValue, ...]:
        _ = request_data
        return tuple(messages)

    @override
    async def async_post_call_success_deployment_hook(  # ast-grep-ignore: no-long-names
        self,
        request_data: JsonMap,
        response: LLMResponseTypes,
        call_type: CallTypes | None,
    ) -> LLMResponseTypes | None:
        data = dict(request_data)
        guardrails = data.get("guardrails")
        match guardrails:
            case list():
                pass
            case str() | bool() | int() | float() | dict() | None:
                return None
        if not self.should_run_guardrail(data, GuardrailEventHooks.post_call):
            return None
        _ = (response, call_type)
        _ = self._deployment_hook_target()
        return None

    @override
    async def async_logging_hook(
        self, kwargs: dict[str, JsonValue], result: JsonValue, call_type: str
    ) -> tuple[dict[str, JsonValue], JsonValue]:
        if not self.uses_apply_guardrail_interface():
            return kwargs, result
        if not self._event_hook_is_event_type(GuardrailEventHooks.logging_only):
            return kwargs, result
        LOGGER.debug(
            "Guardrail %s: no logging_only scan for call_type=%s in this build",
            self.guardrail_name,
            call_type,
        )
        return kwargs, result

    async def _scan_logged_call(
        self,
        kwargs: dict[str, JsonValue],
        response: JsonValue,
        *,
        translation: JsonValue,
        output_translation: JsonValue,
        scratch_metadata: dict[str, JsonValue],
    ) -> None:
        _ = (kwargs, response, translation, output_translation, scratch_metadata)

    def supports_scan_only_tool_results(self) -> bool:
        return True

    def structured_messages_cover_full_request(self) -> bool:  # ast-grep-ignore: no-long-names
        return False
