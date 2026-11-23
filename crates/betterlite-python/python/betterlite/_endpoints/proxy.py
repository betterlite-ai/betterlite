import logging
from collections.abc import Mapping
from typing import Final, Protocol, runtime_checkable

import betterlite
from betterlite._completion import arguments

LOGGER: Final = logging.getLogger("LiteLLM")


@runtime_checkable
class ProxyAuthProvider(Protocol):
    def get_auth_headers(self) -> Mapping[str, str]: ...


# A proxy's headers go on every family's request; the typed seam stands in for the untyped global
def proxy_headers() -> dict[str, str]:
    provider: ProxyAuthProvider | None = betterlite.proxy_auth
    if provider is None:
        provider = arguments.proxy_auth
    if provider is None:
        return {}
    try:
        return dict(provider.get_auth_headers())
    except Exception as error:
        LOGGER.warning("Failed to get proxy auth headers: %s", error, exc_info=True)
        return {}
