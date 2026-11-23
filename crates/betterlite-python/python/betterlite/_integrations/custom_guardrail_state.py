import hashlib
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
    strict_guardrail_modes,
)
from betterlite._integrations.custom_logger import LOGGER, CustomLogger
from betterlite._integrations.guardrail_types import (
    GuardrailCache,
    GuardrailConfigModel,
    GuardrailValue,
)
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
    records_own_guardrail_information: bool = False
    _guardrail_self_recorded: bool = False

    @override
    def __init__(
        self,
        guardrail_name: str | None = None,
        supported_event_hooks: list[GuardrailEventHooks] | None = None,
        event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None = None,
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
        if timeout is not None:
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
        self._native = native_handle(self, strict=strict)
        super().__init__(**kwargs)

    def uses_apply_guardrail_interface(self) -> bool:
        raise NotImplementedError

    def _validate_event_hook(
        self,
        event_hook: GuardrailEventHooks | list[GuardrailEventHooks] | Mode | None,
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
        return render_message(self._native, default, texts)

    def raise_passthrough_exception(
        self,
        violation_message: str,
        request_data: dict[str, JsonValue],
        detection_info: dict[str, JsonValue] | None = None,
        original_response: GuardrailValue = None,
    ) -> NoReturn:
        match request_data.get("model"):
            case str() as model_name:
                model = model_name
            case bool() | int() | float() | list() | dict() | None:
                model = "unknown"
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

    def raise_sensitive_data_route_exception(
        self,
        route_to_model: str,
        request_data: dict[str, JsonValue],
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

    async def _cached_text_hashes(self, cache: GuardrailCache, session_id: str) -> set[str]:
        key = self._scanned_texts_cache_key(session_id)
        try:
            cached = await cache.async_get_cache(key=key)
        except Exception:
            LOGGER.warning(
                "Guardrail %s: could not read the scanned-texts cache", self.guardrail_name,
                exc_info=True,
            )
            return set()
        return self._hash_set(cached)

    async def filter_new_texts_for_session(
        self,
        texts: list[str] | None,
        request_data: dict[str, JsonValue],
        cache: GuardrailCache,
    ) -> list[str] | None:
        if not self.only_scan_new_messages or not texts:
            return texts
        if self.mask_request_content or self.mask_response_content:
            LOGGER.warning(
                "Guardrail %s: only_scan_new_messages cannot be combined with "
                "mask_request_content or mask_response_content",
                self.guardrail_name,
            )
            return None
        session_id = self._get_session_id_from_request_data(request_data)
        if not session_id:
            return texts
        seen = await self._cached_text_hashes(cache, session_id)
        return [text for text in texts if self._scanned_text_hash(text) not in seen]

    async def mark_texts_scanned(
        self,
        texts: list[str],
        request_data: dict[str, JsonValue],
        cache: GuardrailCache,
    ) -> None:
        if not self.only_scan_new_messages or not texts:
            return
        if self.mask_request_content or self.mask_response_content:
            LOGGER.warning(
                "Guardrail %s: only_scan_new_messages cannot be combined with "
                "mask_request_content or mask_response_content",
                self.guardrail_name,
            )
            return
        session_id = self._get_session_id_from_request_data(request_data)
        if not session_id:
            return
        seen = await self._cached_text_hashes(cache, session_id)
        hashes = [self._scanned_text_hash(text) for text in texts]
        merged = list(dict.fromkeys([*sorted(seen), *hashes]))
        key = self._scanned_texts_cache_key(session_id)
        try:
            await cache.async_set_cache(
                key=key, value=merged, ttl=GUARDRAIL_SCANNED_MESSAGES_CACHE_TTL_SECONDS
            )
        except Exception:
            LOGGER.warning(
                "Guardrail %s: could not write the scanned-texts cache", self.guardrail_name,
                exc_info=True,
            )

    def should_route_on_sensitive_data(self) -> bool:
        return self.on_sensitive_data == "route" and self.sensitive_data_route_to_model is not None

    def handle_sensitive_data_detection(
        self,
        request_data: dict[str, JsonValue],
        detection_info: dict[str, JsonValue] | None = None,
    ) -> NoReturn:
        if self.should_route_on_sensitive_data():
            route = self.sensitive_data_route_to_model
            match route:
                case str():
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
                case None:
                    pass
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
        masked = content_string[:start_index] + mask_string + content_string[end_index:]
        return masked

    @staticmethod
    def get_config_model() -> type[GuardrailConfigModel] | None:
        return None

    @classmethod
    def get_supported_event_hooks(cls) -> list[GuardrailEventHooks] | None:
        _ = cls
        return None
