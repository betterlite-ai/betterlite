import logging
from typing import ClassVar, Final

from pydantic import JsonValue

from betterlite._integrations.custom_logger_base64 import CustomLoggerBase64
from betterlite._integrations.custom_logger_events import CustomLoggerEvents
from betterlite._integrations.custom_logger_hooks import CustomLoggerHooks
from betterlite._integrations.custom_logger_proxy_hooks import CustomLoggerProxyHooks
from betterlite._integrations.custom_logger_redaction import CustomLoggerRedaction
from betterlite.types.litellm_params_state import JsonMap

LOGGER: Final = logging.getLogger("LiteLLM")


class CustomLogger(
    CustomLoggerHooks,
    CustomLoggerProxyHooks,
    CustomLoggerEvents,
    CustomLoggerRedaction,
    CustomLoggerBase64,
):
    server_fulfilled_tool_names: ClassVar[frozenset[str]] = frozenset()
    enforces_request_content: bool = False

    def __init__(
        self,
        turn_off_message_logging: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L89
        message_logging: bool = True,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/custom_logger.py#L91
        **kwargs: JsonValue,
    ) -> None:
        _ = kwargs
        self.message_logging = message_logging
        self.turn_off_message_logging = turn_off_message_logging

    @staticmethod
    def get_callback_env_vars(callback_name: str | None = None) -> list[str]:
        # this build has no Python callback registry (B189, B256), so no name resolves
        _ = callback_name
        return []

    # ast-grep-ignore: no-long-names
    async def get_proxy_server_request_from_cold_storage_with_object_key(
        self, object_key: str
    ) -> JsonMap | None: ...
    def handle_callback_failure(self, callback_name: str) -> None:
        LOGGER.debug(
            "No callback with increment_callback_logging_failure method found for %s. %s",
            callback_name,
            "Ensure 'prometheus' is in your callbacks config.",
        )
