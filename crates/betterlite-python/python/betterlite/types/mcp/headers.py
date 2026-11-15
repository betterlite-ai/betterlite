import re
from collections.abc import Awaitable, Callable, Mapping, MutableMapping
from dataclasses import dataclass
from types import MappingProxyType
from typing import Final, Protocol
from urllib.parse import urlsplit

from typing_extensions import override

DEFAULT_CREDENTIAL_HEADER: Final = "Authorization"

_HEADER_NAME_TOKEN: Final = re.compile(r"^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$")
_DEFAULT_PORTS: Final[Mapping[str, int]] = MappingProxyType({"http": 80, "https": 443})


class Stringable(Protocol):
    @override
    def __str__(self) -> str: ...


class RedirectRequest(Protocol):
    @property
    def headers(self) -> MutableMapping[str, str]: ...

    @property
    def url(self) -> Stringable: ...


def normalize_upstream_header_name(raw: str) -> str | None:
    stripped: Final = raw.strip()
    return stripped if stripped and _HEADER_NAME_TOKEN.match(stripped) else None


def same_header(name: str, other: str) -> bool:
    lowered: Final = name.lower()
    return lowered == other.lower()


def has_header(headers: Mapping[str, str] | None, name: str) -> bool:
    if headers is None:
        return False
    return any(same_header(key, name) for key in headers)


def without_header(headers: Mapping[str, str] | None, name: str) -> dict[str, str] | None:
    if not headers:
        return None
    filtered: Final = {key: value for key, value in headers.items() if not same_header(key, name)}
    if not filtered:
        return None
    return filtered


def crosses_origin(configured: str, target: str) -> bool:
    configured_parts: Final = urlsplit(configured)
    target_parts: Final = urlsplit(target)
    configured_port: Final = configured_parts.port or _DEFAULT_PORTS.get(configured_parts.scheme)
    target_port: Final = target_parts.port or _DEFAULT_PORTS.get(target_parts.scheme)
    if (
        configured_parts.scheme == target_parts.scheme
        and configured_parts.hostname == target_parts.hostname
        and configured_port == target_port
    ):
        return False
    return not (
        configured_parts.hostname == target_parts.hostname
        and configured_parts.scheme == "http"
        and configured_port == _DEFAULT_PORTS["http"]
        and target_parts.scheme == "https"
        and target_port == _DEFAULT_PORTS["https"]
    )


def custom_credential_slot(headers: Mapping[str, str] | None) -> str | None:
    if headers is None:
        return None
    for name in headers:
        if not same_header(name, DEFAULT_CREDENTIAL_HEADER):
            return name
    return None


@dataclass(frozen=True)
class CredentialRedirectGuard:
    configured_url: str
    slot: str

    async def __call__(self, request: RedirectRequest) -> None:
        if self.slot in request.headers and crosses_origin(self.configured_url, str(request.url)):
            del request.headers[self.slot]


def credential_redirect_hook(
    configured_url: str, slot: str | None
) -> Callable[[RedirectRequest], Awaitable[None]] | None:
    if not configured_url or not slot or same_header(slot, DEFAULT_CREDENTIAL_HEADER):
        return None
    return CredentialRedirectGuard(configured_url, slot)
