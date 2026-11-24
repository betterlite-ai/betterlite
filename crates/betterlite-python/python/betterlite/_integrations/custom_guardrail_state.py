import hashlib
import json
from collections.abc import Mapping
from typing import NoReturn

from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.custom_guardrail_constants import (
    GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS,
    NO_SESSION_ROUTING_MESSAGE,
)
from betterlite._integrations.custom_guardrail_request import session_id_from_request_data
from betterlite._integrations.custom_guardrail_support import (
    advisory_message,
    json_projection,
    native_handle,
    render_message,
    settings_of,
    strict_guardrail_modes,
    text_value,
)
from betterlite._integrations.custom_logger import LOGGER, CustomLogger
from betterlite._integrations.guardrail_types import (
    GuardrailCache,
    GuardrailConfigModel,
    GuardrailValue,
)
from betterlite._native import NativeGuardrail
from betterlite.exceptions._process import (
    GuardrailRaisedException,
    ModifyResponseException,
    SensitiveDataRouteException,
)
from betterlite.types.guardrails.params import GuardrailEventHooks, Mode


class GuardrailState(CustomLogger):
    timeout: float | None = None
    use_native_during_call_hook: bool = False
    use_native_lifecycle_hooks: bool = False
    # ast-grep-ignore: no-long-names
    records_own_guardrail_information: bool = False
    _guardrail_self_recorded: bool = False
    _native_cache: tuple[str, NativeGuardrail] | None = None

    @override
    def __init__(
        self,
        guardrail_name: str | None = None,
        supported_event_hooks: list[GuardrailEventHooks] | None = None,
        event_hook: GuardrailEventHooks | str | list[GuardrailEventHooks] | Mode | None = None,
        default_on: bool = False,
        mask_request_content: bool = False,
        mask_response_content: bool = False,
        violation_message_template: str | None = None,
        end_session_after_n_fails: int | None = None,
        on_violation: str | None = None,
        realtime_violation_message: str | None = None,
        on_sensitive_data: str | None = None,
        sensitive_data_route_to_model: str | None = None,
        sticky_session_routing: bool = True,
        run_in_parallel: bool = False,
        scan_raw_request: bool = False,
        only_scan_new_messages: bool = False,
        timeout: float | None = None,
        **kwargs: bool,
    ) -> None:
        self.guardrail_name = guardrail_name
        self.supported_event_hooks = supported_event_hooks
        self.event_hook = event_hook
        self.default_on = default_on
        self.mask_request_content = mask_request_content
        self.mask_response_content = mask_response_content
        self.violation_message_template = violation_message_template
        self.end_session_after_n_fails = end_session_after_n_fails
        self.on_violation = on_violation
        self.realtime_violation_message = realtime_violation_message
        self.on_sensitive_data = on_sensitive_data
        self.sensitive_data_route_to_model = sensitive_data_route_to_model
        self.sticky_session_routing = sticky_session_routing
        self.run_in_parallel = run_in_parallel
        self.scan_raw_request = scan_raw_request
        self.only_scan_new_messages = only_scan_new_messages
        self.timeout = timeout
        strict = strict_guardrail_modes()
        hooks = self.supported_event_hooks
        if hooks:
            try:
                self._validate_event_hook(self.event_hook, hooks)
            except ValueError as error:
                if strict:
                    raise
                LOGGER.warning(
                    "Guardrail %s: unsupported event_hook %s (%s). "
                    "LITELLM_STRICT_GUARDRAIL_MODES=false, so startup continues.",
                    self.guardrail_name,
                    self.event_hook,
                    error,
                )
        super().__init__(**kwargs)

    def _native(self) -> NativeGuardrail:
        text = json.dumps(settings_of(self))
        cached = self._native_cache
        if cached is None or cached[0] != text:
            cached = (text, native_handle(self, strict=False))
            self._native_cache = cached
        return cached[1]

    def uses_apply_guardrail_interface(self) -> bool:
        raise NotImplementedError

    def _validate_event_hook(
        self,
        event_hook: GuardrailEventHooks | str | list[GuardrailEventHooks] | Mode | None,
        supported_event_hooks: list[GuardrailEventHooks],
    ) -> None:
        _ = (event_hook, supported_event_hooks)
        raise NotImplementedError

    def render_violation_message(
        self, default: str, context: Mapping[str, JsonValue] | None = None
    ) -> str:
        template = self.violation_message_template
        if not template:
            return default
        texts: dict[str, str] = {"default_message": default}
        if context is not None:
            for name, value in context.items():
                texts[name] = str(value)
        return render_message(self._native(), default, texts)

    def raise_passthrough_exception(
        self,
        violation_message: str,
        request_data: dict[str, JsonValue],
        detection_info: dict[str, JsonValue] | None = None,
        original_response: GuardrailValue = None,
    ) -> NoReturn:
        model = text_value(request_data.get("model"), "unknown")
        raise ModifyResponseException(
            message=violation_message,
            model=model,
            request_data=request_data,
            guardrail_name=self.guardrail_name,
            detection_info=detection_info,
            original_response=json_projection(original_response),
        )

    def inject_advisory_message(self, data: dict[str, JsonValue], message: str) -> bool:
        written, updated = advisory_message(data, message)
        for key in ("messages", "input", "instructions"):
            if key in updated:
                data[key] = updated[key]
        return written

    # ast-grep-ignore: no-long-names
    def raise_sensitive_data_route_exception(
        self, route_to_model: str, request_data: dict[str, JsonValue],
        detection_info: dict[str, JsonValue] | None = None,
    ) -> NoReturn:
        session_id = self._get_session_id_from_request_data(request_data)
        if session_id is None:
            raise ValueError(NO_SESSION_ROUTING_MESSAGE)
        raise SensitiveDataRouteException(
            route_to_model=route_to_model,
            session_id=session_id,
            guardrail_name=self.guardrail_name,
            detection_info=detection_info,
            sticky_session_routing=self.sticky_session_routing,
        )

    # ast-grep-ignore: no-long-names
    def _get_session_id_from_request_data(self, request_data: dict[str, JsonValue]) -> str | None:
        return session_id_from_request_data(request_data)

    @staticmethod
    def _scanned_text_hash(text: str) -> str:
        return hashlib.sha256(text.encode("utf-8")).hexdigest()

    def _scanned_texts_cache_key(self, session_id: str) -> str:
        return f"guardrail_scanned_texts:{self.guardrail_name}:{session_id}"

    def _hash_set(self, cached: JsonValue) -> set[str]:
        match cached:
            case list():
                return {str(item) for item in cached}
            case str() | bool() | int() | float() | dict() | None:
                return set()

    async def _cached_hashes(self, cache: GuardrailCache, session_id: str) -> set[str] | None:
        key = self._scanned_texts_cache_key(session_id)
        try:
            cached = await cache.async_get_cache(key=key)
        except Exception as error:
            LOGGER.warning("Guardrail %s: read cache failed: %s", self.guardrail_name, error)
            return None
        return self._hash_set(cached)

    async def _scan_baseline(
        self, texts: list[str] | None, request_data: dict[str, JsonValue],
        cache: GuardrailCache, *, warn_masking: bool,
    ) -> tuple[list[str], str, set[str]] | None:
        if not self.only_scan_new_messages or not texts:
            return None
        if self.mask_request_content or self.mask_response_content:
            if warn_masking:
                LOGGER.warning(
                    "Guardrail %s: only_scan_new_messages is ignored while masking is on",
                    self.guardrail_name,
                )
            return None
        session_id = self._get_session_id_from_request_data(request_data)
        if not session_id:
            return None
        seen = await self._cached_hashes(cache, session_id)
        if seen is None:
            return None
        return texts, session_id, seen

    async def filter_new_texts_for_session(
        self, texts: list[str] | None, request_data: dict[str, JsonValue], cache: GuardrailCache,
    ) -> list[str] | None:
        baseline = await self._scan_baseline(texts, request_data, cache, warn_masking=True)
        if baseline is None:
            return None
        texts, _, seen = baseline
        return [text for text in texts if self._scanned_text_hash(text) not in seen]

    async def mark_texts_scanned(
        self, texts: list[str] | None, request_data: dict[str, JsonValue], cache: GuardrailCache,
    ) -> None:
        baseline = await self._scan_baseline(texts, request_data, cache, warn_masking=False)
        if baseline is None:
            return
        texts, session_id, seen = baseline
        merged: list[str] = sorted(seen | {self._scanned_text_hash(text) for text in texts})
        key = self._scanned_texts_cache_key(session_id)
        try:
            await cache.async_set_cache(
                # ast-grep-ignore: no-long-names
                key=key, value=merged, ttl=GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS
            )
        except Exception as error:
            LOGGER.warning("Guardrail %s: write cache failed: %s", self.guardrail_name, error)

    def should_route_on_sensitive_data(self) -> bool:
        return self.on_sensitive_data == "route" and self.sensitive_data_route_to_model is not None

    # ast-grep-ignore: no-long-names
    def handle_sensitive_data_detection(
        self, request_data: dict[str, JsonValue],
        detection_info: dict[str, JsonValue] | None = None,
    ) -> NoReturn:
        route = self.sensitive_data_route_to_model
        if self.should_route_on_sensitive_data() and route is not None:
            try:
                self.raise_sensitive_data_route_exception(
                    route_to_model=route,
                    request_data=request_data,
                    detection_info=detection_info,
                )
            except ValueError as error:
                raise GuardrailRaisedException(
                    message=(
                        f"Sensitive data detected by {self.guardrail_name} "
                        "(routing skipped: request has no session_id)"
                    ),
                    guardrail_name=self.guardrail_name,
                    blocked_content=True,
                ) from error
        raise GuardrailRaisedException(
            message=f"Sensitive data detected by {self.guardrail_name}",
            guardrail_name=self.guardrail_name,
            blocked_content=True,
        )

    def mask_content_in_string(
        self, content_string: str, mask_string: str, start_index: int, end_index: int
    ) -> str:
        if not (0 <= start_index < end_index <= len(content_string)):
            return content_string
        return content_string[:start_index] + mask_string + content_string[end_index:]

    @staticmethod
    def get_config_model() -> type[GuardrailConfigModel] | None:
        return None

    @classmethod
    def get_supported_event_hooks(cls) -> list[GuardrailEventHooks] | None:
        _ = cls
        return None
