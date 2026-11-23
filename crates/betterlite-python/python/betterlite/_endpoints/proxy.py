import logging
from collections.abc import Mapping
from typing import Final, Protocol, runtime_checkable

import betterlite

LOGGER: Final = logging.getLogger("LiteLLM")


@runtime_checkable
class ProxyAuthProvider(Protocol):
    def get_auth_headers(self) -> Mapping[str, str]: ...


# A proxy's headers go on every family's request; a failing one is warned about and skipped
def proxy_headers() -> dict[str, str]:
    provider: ProxyAuthProvider | None = betterlite.proxy_auth
    if provider is None:
        return {}
    try:
        return dict(provider.get_auth_headers())
    except Exception as error:
        LOGGER.warning("Failed to get proxy auth headers: %s", error, exc_info=True)
        return {}
