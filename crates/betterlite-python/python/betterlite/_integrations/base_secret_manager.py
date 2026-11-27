import logging
import re
from abc import ABC, abstractmethod
from typing import Final, Protocol, TypeAlias

from pydantic import JsonValue

LOGGER: Final = logging.getLogger("LiteLLM")
_UNSAFE_SECRET_NAME_PATTERN: Final = re.compile(
    r"(^|/)\.\.(/|$)|[\x00-\x1f\x7f-\x9f\x85\u2028\u2029]"
)


class HttpTimeout(Protocol):
    connect: float | None
    read: float | None
    write: float | None
    pool: float | None


RequestTimeout: TypeAlias = float | HttpTimeout | None


def raise_if_unsafe_secret_name(secret_name: str) -> None:
    if _UNSAFE_SECRET_NAME_PATTERN.search(secret_name) is not None:
        raise ValueError(f"Invalid secret_name {secret_name!r}")


class BaseSecretManager(ABC):
    @abstractmethod
    async def async_read_secret(
        self,
        secret_name: str,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
    ) -> str | None: ...

    @abstractmethod
    def sync_read_secret(
        self,
        secret_name: str,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
    ) -> str | None: ...

    @abstractmethod
    async def async_write_secret(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/secret_managers/base_secret_manager.py#L68
        self,
        secret_name: str,
        secret_value: str,
        description: str | None = None,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
        tags: dict[str, JsonValue] | list[JsonValue] | None = None,
    ) -> dict[str, JsonValue]: ...

    @abstractmethod
    async def async_delete_secret(
        self,
        secret_name: str,
        recovery_window_in_days: int | None = 7,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
    ) -> dict[str, JsonValue]: ...

    async def async_rotate_secret(
        self,
        current_secret_name: str,
        new_secret_name: str,
        new_secret_value: str,
        optional_params: dict[str, JsonValue] | None = None,
        timeout: RequestTimeout = None,
    ) -> dict[str, JsonValue]:
        try:
            old_secret: Final = await self.async_read_secret(
                secret_name=current_secret_name, optional_params=optional_params, timeout=timeout
            )
            if old_secret is None:
                raise ValueError(f"Current secret {current_secret_name} not found")
            created: Final = await self.async_write_secret(
                secret_name=new_secret_name,
                secret_value=new_secret_value,
                description=f"Rotated from {current_secret_name}",
                optional_params=optional_params,
                timeout=timeout,
            )
            new_secret: Final = await self.async_read_secret(
                secret_name=new_secret_name, optional_params=optional_params, timeout=timeout
            )
            if new_secret is None:
                raise ValueError(f"Failed to verify new secret {new_secret_name}")
            await self.async_delete_secret(
                secret_name=current_secret_name,
                recovery_window_in_days=7,
                optional_params=optional_params,
                timeout=timeout,
            )
        except Exception:
            LOGGER.exception("Error rotating secret in AWS Secrets Manager")
            raise
        return created
