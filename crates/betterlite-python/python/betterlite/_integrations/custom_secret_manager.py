import logging
from abc import abstractmethod
from typing import Final

from pydantic import JsonValue
from typing_extensions import override

from betterlite._integrations.base_secret_manager import BaseSecretManager, RequestTimeout

LOGGER: Final = logging.getLogger("LiteLLM")
DEFAULT_MANAGER_NAME: Final = "custom_secret_manager"


def _unsupported(operation: str, manager_name: str) -> NotImplementedError:
    return NotImplementedError(
        f"{operation.capitalize()} operations are not implemented for {manager_name}. "
        f"Override async_{operation}_secret() to add {operation} support."
    )


class CustomSecretManager(BaseSecretManager):
    @override
    def __init__(self, secret_manager_name: str | None = None, **kwargs: JsonValue) -> None:
        super().__init__()
        self.secret_manager_name = (
            DEFAULT_MANAGER_NAME if secret_manager_name is None else secret_manager_name
        )
        LOGGER.info("Initialized custom secret manager")

    @override
    @abstractmethod
    async def async_read_secret(
        self,
        secret_name: str,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
    ) -> str | None: ...

    @override
    @abstractmethod
    def sync_read_secret(
        self,
        secret_name: str,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
    ) -> str | None: ...

    @override
    async def async_write_secret(
        self,
        secret_name: str,
        secret_value: str,
        description: str | None = None,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
        tags: dict[str, JsonValue] | list[JsonValue] | None = None,
    ) -> dict[str, JsonValue]:
        raise _unsupported("write", self.secret_manager_name)

    @override
    async def async_delete_secret(
        self,
        secret_name: str,
        recovery_window_in_days: int | None = 7,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
    ) -> dict[str, JsonValue]:
        raise _unsupported("delete", self.secret_manager_name)

    def validate_environment(self) -> bool:
        LOGGER.debug("No environment validation configured for custom secret manager")
        return True

    async def async_health_check(
        self,
        timeout: RequestTimeout = None,
    ) -> bool:
        _ = timeout
        LOGGER.debug("Health check not implemented for %s", self.secret_manager_name)
        return True

    @override
    def __repr__(self) -> str:
        return f"<{self.__class__.__name__}(name={self.secret_manager_name})>"
