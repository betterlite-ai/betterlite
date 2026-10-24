import asyncio
import base64
import os
import tempfile
from typing import TypeAlias

import httpx
import litellm
from aiohttp import BasicAuth
from aiohttp.helpers import get_env_proxy_for_url
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from yarl import URL

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def describe(proxy: URL, auth: BasicAuth | None) -> Json:
    header = auth.encode().removeprefix("Basic ") if auth else None
    pair = base64.b64decode(header).decode("latin1") if header else None
    return {"scheme": proxy.scheme, "host": proxy.host, "port": proxy.port, "credentials": pair}


async def pick(url: str) -> Json:
    # a fresh transport per case, since its proxy_cache keeps the first pick per host
    transport = AsyncHTTPHandler._create_aiohttp_transport()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    proxy = await transport._get_proxy_settings(httpx.Request("GET", url))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if proxy is not None:
        target = URL(proxy)
        has_auth = target.raw_user or target.raw_password
        auth = BasicAuth(target.user or "", target.password or "") if has_auth else None
        return describe(target, auth)
    session = transport._client_factory()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        if not session.trust_env:
            return None
        found, auth = get_env_proxy_for_url(URL(url))
        return describe(found, auth)
    except LookupError:
        return None
    finally:
        await session.close()


def run(input_value: dict[str, Json]) -> Json:
    match input_value:
        case {
            "url": str(url),
            "environment": dict(environment),
            "aiohttp_trust_env": bool(trust_env),
            "disable_aiohttp_trust_env": bool(disable_trust_env),
        } if len(input_value) == 4:  # noqa: PLR2004 - the four fields above
            pass
        case _:
            raise ValueError("proxy_policy input has missing, extra or mistyped fields")
    variables = {name: value for name, value in environment.items() if isinstance(value, str)}
    if len(variables) != len(environment):
        raise ValueError("environment values must be strings")
    litellm.aiohttp_trust_env = trust_env
    litellm.disable_aiohttp_trust_env = disable_trust_env
    with tempfile.TemporaryDirectory() as home:
        # an empty HOME keeps the host's ~/.netrc out of the comparison
        os.environ.clear()
        os.environ["HOME"] = home
        os.environ.update(variables)
        return {"proxy": asyncio.run(pick(url))}
