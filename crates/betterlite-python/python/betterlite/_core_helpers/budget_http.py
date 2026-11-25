import ssl
from collections.abc import Mapping
from contextlib import suppress
from http.cookiejar import CookieJar, DefaultCookiePolicy
from pathlib import Path
from typing import Final

import httpx2
from pydantic import JsonValue, TypeAdapter

import betterlite
from betterlite._settings.env import env_str
from betterlite._version import version

_TIMEOUT_SECONDS: Final = 600.0
_CONNECT_SECONDS: Final = 5.0
_USER_AGENT_VARIABLE: Final = "LITELLM_USER_AGENT"
_SSL_CERTIFICATE_VARIABLE: Final = "SSL_CERTIFICATE"
_DEFAULT_TIMEOUT: Final = httpx2.Timeout(timeout=_TIMEOUT_SECONDS, connect=_CONNECT_SECONDS)
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
    match ssl_verify:
        case bool() | str():
            resolved = ssl_verify
        case None:
            text = env_str("SSL_VERIFY", "")
            resolved = text if text != "" else betterlite.ssl_verify
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
    match security_level:
        case str() as level if level != "":
            context.set_ciphers(level)
        case str() | None:
            context.set_ciphers(DEFAULT_SSL_CIPHERS)
    match ecdh_curve:
        case str() as curve:
            with suppress(AttributeError, ValueError):
                context.set_ecdh_curve(curve)
        case None:
            pass
    return context


def ssl_configuration(
    *, ssl_verify: bool | str | ssl.SSLContext | None = None
) -> bool | str | ssl.SSLContext:
    match ssl_verify:
        case ssl.SSLContext():
            return ssl_verify
        case bool() | str() | None:
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


class ModuleClient:
    client: Final[httpx2.Client]
    timeout: Final[httpx2.Timeout]
    ssl_options: Final[bool | str | ssl.SSLContext]

    def __init__(
        self,
        *,
        timeout: httpx2.Timeout | None = None,
        ssl_verify: bool | str | ssl.SSLContext | None = None,
    ) -> None:
        certificate: str | None = env_str(_SSL_CERTIFICATE_VARIABLE, "")
        if certificate == "":
            certificate = betterlite.ssl_certificate
        agent = env_str(_USER_AGENT_VARIABLE, f"betterlite/{version}")
        self.timeout = _DEFAULT_TIMEOUT if timeout is None else timeout
        self.ssl_options = ssl_configuration(ssl_verify=ssl_verify)
        self.client = httpx2.Client(
            timeout=self.timeout,
            follow_redirects=True,
            headers={"User-Agent": agent},
            cookies=CookieJar(policy=DefaultCookiePolicy(allowed_domains=())),
            verify=self.ssl_options,
            cert=certificate,
            http2=betterlite.http2,
        )

    def post(self, url: str, *, headers: Mapping[str, str], json: JsonValue) -> JsonValue:
        response = self.client.post(url, headers=dict(headers), json=json)
        response.raise_for_status()
        return _JSON.validate_json(response.text)
