import ssl
from collections.abc import Mapping
from contextlib import suppress
from http.cookiejar import CookieJar, DefaultCookiePolicy
from pathlib import Path
from typing import Final

import httpx2
from httpx2._utils import get_environment_proxies
from pydantic import JsonValue, TypeAdapter

import betterlite
from betterlite._settings.env import env_number, env_str
from betterlite._version import version

_DEFAULT_REQUEST_SECONDS: Final = 6000.0
_REQUEST_TIMEOUT_VARIABLE: Final = "REQUEST_TIMEOUT"
_HTTP2_VARIABLE: Final = "LITELLM_HTTP2"
_IPV4_LOCAL_ADDRESS: Final = "0.0.0.0"
_USER_AGENT_VARIABLE: Final = "LITELLM_USER_AGENT"
_SSL_CERTIFICATE_VARIABLE: Final = "SSL_CERTIFICATE"
_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
DEFAULT_SSL_CIPHERS: Final = env_str(
    "LITELLM_SSL_CIPHERS",
    "TLS_AES_256_GCM_SHA384:TLS_AES_128_GCM_SHA256:TLS_CHACHA20_POLY1305_SHA256:"
    "ECDHE-RSA-AES256-GCM-SHA384:ECDHE-RSA-AES128-GCM-SHA256:"
    "ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-ECDSA-AES128-GCM-SHA256:"
    "ECDHE-RSA-CHACHA20-POLY1305:ECDHE-ECDSA-CHACHA20-POLY1305:"
    "ECDHE-RSA-AES256-SHA384:ECDHE-RSA-AES128-SHA256:"
    "AES256-GCM-SHA384:AES128-GCM-SHA256",
)


def ssl_verify_setting(*, ssl_verify: bool | str | None = None) -> bool | str:
    resolved: bool | str
    if ssl_verify is None:
        text = env_str("SSL_VERIFY", "")
        resolved = text if text != "" else betterlite.ssl_verify
    else:
        resolved = ssl_verify
    match resolved:
        case str() as text:
            if Path(text).exists():
                return text
            normalized = text.strip().lower()
            if normalized == "true":
                resolved = True
            elif normalized == "false":
                resolved = False
        case bool():
            pass
    if resolved is True:
        cert_file = env_str("SSL_CERT_FILE", "")
        if cert_file != "" and Path(cert_file).exists():
            return cert_file
    return resolved


def _configured_setting(variable: str, fallback: str | None) -> str | None:
    text = env_str(variable, "")
    if text == "":
        return fallback
    return text


def _ssl_context(
    *, cafile: str | None, security_level: str | None, ecdh_curve: str | None
) -> ssl.SSLContext:
    context = ssl.create_default_context(cafile=cafile)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    if security_level:
        context.set_ciphers(security_level)
    else:
        context.set_ciphers(DEFAULT_SSL_CIPHERS)
    if ecdh_curve is not None:
        with suppress(AttributeError, ValueError):
            context.set_ecdh_curve(ecdh_curve)
    return context


def ssl_configuration(*, ssl_verify: bool | str | None = None) -> bool | str | ssl.SSLContext:
    resolved = ssl_verify_setting(ssl_verify=ssl_verify)
    security_level = _configured_setting("SSL_SECURITY_LEVEL", betterlite.ssl_security_level)
    curve = _configured_setting("SSL_ECDH_CURVE", betterlite.ssl_ecdh_curve)
    cafile: str | None = None
    match resolved:
        case str() as text:
            if Path(text).exists():
                cafile = text
        case bool():
            pass
    if cafile is None:
        cert_file = env_str("SSL_CERT_FILE", "")
        if cert_file != "" and Path(cert_file).exists():
            cafile = cert_file
    if resolved is False:
        return False
    return _ssl_context(cafile=cafile, security_level=security_level, ecdh_curve=curve)


def _default_timeout() -> httpx2.Timeout:
    return httpx2.Timeout(
        timeout=env_number(_REQUEST_TIMEOUT_VARIABLE, _DEFAULT_REQUEST_SECONDS, float)
    )


def _http2_enabled() -> bool:
    if betterlite.http2 is True:
        return True
    return env_str(_HTTP2_VARIABLE, "False").strip().lower() == "true"


def _sync_transport(*, http2: bool) -> httpx2.HTTPTransport | None:
    if betterlite.force_ipv4:
        return httpx2.HTTPTransport(local_address=_IPV4_LOCAL_ADDRESS, http2=http2)
    return None


def _proxy_mounts(
    *, http2: bool, ssl_options: bool | str | ssl.SSLContext, certificate: str | None
) -> dict[str, httpx2.HTTPTransport | None] | None:
    if not betterlite.force_ipv4:
        return None
    mounts: dict[str, httpx2.HTTPTransport | None] = {}
    for pattern, proxy_url in get_environment_proxies().items():
        if proxy_url is None:
            mounts[pattern] = None
            continue
        mounts[pattern] = httpx2.HTTPTransport(
            proxy=proxy_url, verify=ssl_options, cert=certificate, http2=http2
        )
    return mounts


class ModuleClient:
    client: Final[httpx2.Client]
    timeout: Final[httpx2.Timeout]
    ssl_options: Final[bool | str | ssl.SSLContext]

    def __init__(
        self,
        *,
        timeout: httpx2.Timeout | None = None,
        ssl_verify: bool | str | None = None,
    ) -> None:
        certificate: str | None = env_str(_SSL_CERTIFICATE_VARIABLE, "")
        if certificate == "":
            certificate = betterlite.ssl_certificate
        agent = env_str(_USER_AGENT_VARIABLE, f"betterlite/{version}")
        http2 = _http2_enabled()
        self.timeout = _default_timeout() if timeout is None else timeout
        self.ssl_options = ssl_configuration(ssl_verify=ssl_verify)
        self.client = httpx2.Client(
            transport=_sync_transport(http2=http2),
            mounts=_proxy_mounts(
                http2=http2, ssl_options=self.ssl_options, certificate=certificate
            ),
            timeout=self.timeout,
            follow_redirects=True,
            headers={"User-Agent": agent},
            cookies=CookieJar(policy=DefaultCookiePolicy(allowed_domains=())),
            verify=self.ssl_options,
            cert=certificate,
            http2=http2,
        )

    def post(self, url: str, *, headers: Mapping[str, str], json: JsonValue) -> JsonValue:
        response = self.client.post(url, headers=dict(headers), json=json)
        response.raise_for_status()
        return _JSON.validate_json(response.text)
