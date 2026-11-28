import json
from collections.abc import Mapping
from typing import NoReturn

from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.custom_guardrail_constants import STREAM_CONTROL_KEYS
from betterlite._integrations.custom_guardrail_request import (
    apply_log_target,
    apply_records,
    is_guardrail_intervention,
    log_target_json,
)
from betterlite._integrations.custom_guardrail_selection import GuardrailSelection
from betterlite._integrations.custom_guardrail_support import (
    guardrail_summary,
    json_projection,
    record_add,
    settings_of,
)
from betterlite._integrations.custom_logger import LOGGER
from betterlite._integrations.guardrail_types import (
    JSON,
    GuardrailRecorder,
    GuardrailStatus,
    GuardrailTracingDetail,
    GuardrailValue,
    RecordContext,
    RecordInput,
)
from betterlite.types.guardrails.params import GuardrailEventHooks


class GuardrailRecording(GuardrailSelection, GuardrailRecorder):
    @staticmethod
    def _is_guardrail_intervention(e: Exception) -> bool:
        return is_guardrail_intervention(e)

    @override
    # ast-grep-ignore: no-long-names
    def add_standard_logging_guardrail_information_to_request_data(
        self, guardrail_json_response: GuardrailValue, request_data: dict[str, JsonValue],
        guardrail_status: GuardrailStatus, start_time: float | None = None,
        end_time: float | None = None, *, duration: float | None = None,
        masked_entity_count: dict[str, int] | None = None,
        guardrail_provider: str | None = None, event_type: GuardrailEventHooks | None = None,
        tracing_detail: GuardrailTracingDetail | None = None,
    ) -> None:
        detail = None if tracing_detail is None else JSON.validate_json(json.dumps(tracing_detail))
        record = RecordInput(
            guardrail_response=json_projection(guardrail_json_response),
            guardrail_status=guardrail_status,
            start_time=start_time,
            end_time=end_time,
            duration=duration,
            masked_entity_count=masked_entity_count,
            guardrail_provider=guardrail_provider,
            event_type=None if event_type is None else event_type.value,
            tracing_detail=detail,
        )
        logging_obj = request_data.get("litellm_logging_obj")
        context = RecordContext(settings=settings_of(self), log_target=log_target_json(logging_obj))
        updated, target = record_add(request_data, record, context)
        apply_records(request_data, updated)
        apply_log_target(logging_obj, target)
        self._guardrail_self_recorded = True

    @override
    def _process_response(
        self, response: GuardrailValue, request_data: dict[str, JsonValue],
        start_time: float | None = None, end_time: float | None = None,
        *, duration: float | None = None, event_type: GuardrailEventHooks | None = None,
        original_inputs: dict[str, JsonValue] | None = None,
    ) -> GuardrailValue:
        guardrail_response = self._summarize_guardrail_response(
            response, original_inputs, event_type
        )
        LOGGER.debug("Guardrail response: %s", response)
        self.add_standard_logging_guardrail_information_to_request_data(
            guardrail_response, request_data, "success", start_time, end_time,
            duration=duration, event_type=event_type,
        )
        return response

    def _summarize_guardrail_response(
        self,
        response: GuardrailValue,
        original_inputs: Mapping[str, JsonValue] | None,
        event_type: GuardrailEventHooks | None,
    ) -> GuardrailValue:
        baseline = None if original_inputs is None else dict(original_inputs)
        return guardrail_summary(response, baseline, event_type)

    @override
    def _process_error(
        self, e: Exception, request_data: dict[str, JsonValue],
        start_time: float | None = None, end_time: float | None = None,
        *, duration: float | None = None, event_type: GuardrailEventHooks | None = None,
    ) -> NoReturn:
        status: GuardrailStatus = (
            "guardrail_intervened" if self._is_guardrail_intervention(e)
            else "guardrail_failed_to_respond"
        )
        class_name = self.__class__.__name__
        response: GuardrailValue = "deny" if "CustomCodeGuardrail" in class_name else e
        self.add_standard_logging_guardrail_information_to_request_data(
            response, request_data, status, start_time, end_time,
            duration=duration, event_type=event_type,
        )
        raise e

    def _inputs_were_modified(
        self, original_inputs: Mapping[str, JsonValue], response: Mapping[str, JsonValue]
    ) -> bool:
        compared = (original_inputs.keys() | response.keys()) - STREAM_CONTROL_KEYS
        return any(original_inputs.get(key) != response.get(key) for key in compared)
