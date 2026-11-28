from collections.abc import Awaitable, Callable, Mapping, Sequence
from datetime import datetime, timezone
from typing import Literal, TypeAlias, TypeVar

from pydantic import JsonValue
from typing_extensions import Self, override

from betterlite._integrations.custom_guardrail_recording import GuardrailRecording
from betterlite._integrations.custom_guardrail_request import sync_guardrail_info
from betterlite._integrations.custom_logger import LOGGER, CustomLogger
from betterlite._integrations.custom_logger_types import (
    CallTypes,
    LiteLLMLoggingObj,
    LLMResponseTypes,
)
from betterlite._integrations.guardrail_types import OBJECT, GenericGuardrailAPIInputs
from betterlite.types.litellm_params_state import JsonMap

_Owner = TypeVar("_Owner", bound="GuardrailHooks")

ApplyGuardrailHook: TypeAlias = Callable[
    [
        _Owner, GenericGuardrailAPIInputs, dict[str, JsonValue], Literal["request", "response"],
        LiteLLMLoggingObj | None,
    ],
    Awaitable[GenericGuardrailAPIInputs],
]


class GuardrailHooks(GuardrailRecording):
    async def apply_guardrail(
        self, inputs: GenericGuardrailAPIInputs, request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"], logging_obj: LiteLLMLoggingObj | None = None,
    ) -> GenericGuardrailAPIInputs:
        _ = (request_data, input_type, logging_obj)
        return inputs

    @override
    def uses_apply_guardrail_interface(self) -> bool:
        return self.__class__.apply_guardrail is not GuardrailHooks.apply_guardrail

    async def guarded_apply_guardrail(
        self, hook: ApplyGuardrailHook[Self], inputs: GenericGuardrailAPIInputs,
        request_data: dict[str, JsonValue], input_type: Literal["request", "response"],
        logging_obj: LiteLLMLoggingObj | None,
    ) -> GenericGuardrailAPIInputs:
        start = datetime.now(tz=timezone.utc)
        original_inputs = OBJECT.validate_python(inputs)
        fallback = request_data.get("litellm_logging_obj")
        log_obj = fallback if logging_obj is None else logging_obj
        self._guardrail_self_recorded = False
        try:
            response = await hook(self, inputs, request_data, input_type, logging_obj)
            if not (self.records_own_guardrail_information or self._guardrail_self_recorded):
                now = datetime.now(tz=timezone.utc)
                self._process_response(
                    OBJECT.validate_python(response), request_data, start.timestamp(),
                    now.timestamp(), duration=(now - start).total_seconds(), event_type=None,
                    original_inputs=original_inputs,
                )
        except Exception as error:
            LOGGER.debug("Guardrail %s raised", "apply_guardrail", exc_info=True)
            if not self._guardrail_self_recorded:
                now = datetime.now(tz=timezone.utc)
                self._process_error(
                    error, request_data, start.timestamp(), now.timestamp(),
                    duration=(now - start).total_seconds(), event_type=None,
                )
            raise
        else:
            return response
        finally:
            self._guardrail_self_recorded = False
            sync_guardrail_info(request_data, log_obj)

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
        _ = call_type
        return dict(kwargs)

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
        _ = (request_data, response, call_type)
        return None

    @override
    async def async_logging_hook(
        self, kwargs: dict[str, JsonValue], result: JsonValue, call_type: str
    ) -> tuple[dict[str, JsonValue], JsonValue]:
        _ = call_type
        return kwargs, result

    async def _scan_logged_call(
        self, kwargs: dict[str, JsonValue], response: JsonValue, translation: JsonValue,
        output_translation: JsonValue, scratch_metadata: dict[str, JsonValue],
    ) -> None:
        _ = (kwargs, response, translation, output_translation, scratch_metadata)

    # ast-grep-ignore: no-long-names
    def supports_scan_only_tool_results(self) -> bool:
        return True

    def structured_messages_cover_full_request(self) -> bool:  # ast-grep-ignore: no-long-names
        return False
