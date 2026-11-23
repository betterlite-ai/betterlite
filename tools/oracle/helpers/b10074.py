import os
import sys
from collections.abc import Generator
from contextlib import contextmanager
from types import ModuleType
from typing import Final, Literal, TypedDict

from litellm.proxy.common_utils.encrypt_decrypt_utils import _decrypt_with_signing_key  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/encrypt_decrypt_utils.py#L160
from litellm.proxy.proxy_server import ProxyConfig
from pydantic import JsonValue, TypeAdapter

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
SALT_ENV: Final = "LITELLM_SALT_KEY"
AES_PREFIX: Final = "v2:gcm:"

ENV_MAP: Final = TypeAdapter(dict[str, JsonValue])


class Call(TypedDict):
    operation: Literal["encrypt_env", "decrypt_env", "encrypt_env_for_db"]
    values: dict[str, JsonValue]
    new_key: str | None


class Input(TypedDict):
    salt_key: str | None
    master_key: str | None
    general_settings: dict[str, JsonValue]
    calls: list[Call]


class ProxyServer(ModuleType):
    def __init__(
        self,
        name: str,
        master_key: str | None,
        general_settings: dict[str, JsonValue],
    ) -> None:
        super().__init__(name)
        self.master_key = master_key
        self.general_settings = general_settings


@contextmanager
def fake_proxy(master_key: str | None, general_settings: dict[str, JsonValue]) -> Generator[None]:
    proxy_server = ProxyServer(PROXY_MODULE, master_key, general_settings)
    saved = sys.modules.get(PROXY_MODULE)
    sys.modules[PROXY_MODULE] = proxy_server
    try:
        yield
    finally:
        if saved is None:
            del sys.modules[PROXY_MODULE]
        else:
            sys.modules[PROXY_MODULE] = saved


def prepare_environment(input_value: Input) -> None:
    declared = input_value["salt_key"]
    current = os.environ.get(SALT_ENV)
    if current is not None and current != declared:
        raise ValueError("the case env and the helper input disagree on LITELLM_SALT_KEY")
    if declared is not None:
        os.environ[SALT_ENV] = declared


def resolve_write_key(input_value: Input, new_key: str | None) -> str | None:
    if new_key:
        return new_key
    salt_key = os.environ.get(SALT_ENV)
    if salt_key is not None:
        return salt_key
    return input_value["master_key"]


def sealed_view(sealed: dict[str, JsonValue], key: str | None) -> dict[str, JsonValue]:
    view: dict[str, JsonValue] = {}
    for name, value in sealed.items():
        match value:
            case str():
                if key is None:
                    raise ValueError("the write key is missing")
                entry: dict[str, JsonValue] = {
                    "opened": _decrypt_with_signing_key(value, key),  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/encrypt_decrypt_utils.py#L160
                    "v2": value.startswith(AES_PREFIX),
                }
                view[name] = entry
            case None | bool() | int() | float() | list() | dict():
                view[name] = value
    return view


def answer(proxy_config: ProxyConfig, input_value: Input, call: Call) -> dict[str, JsonValue]:
    new_key = call["new_key"]
    key = resolve_write_key(input_value, new_key)
    match call["operation"]:
        case "decrypt_env":
            return ENV_MAP.validate_python(proxy_config._decrypt_db_variables(call["values"]))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7614-L7622
        case "encrypt_env":
            sealed = ENV_MAP.validate_python(proxy_config._encrypt_env_variables(call["values"], new_key))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7575-L7583
            return sealed_view(sealed, key)
        case "encrypt_env_for_db":
            sealed = ENV_MAP.validate_python(proxy_config._encrypt_env_variables_for_db(call["values"], new_key))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7624-L7643
            return sealed_view(sealed, key)


def call_answer(proxy_config: ProxyConfig, input_value: Input, call: Call) -> JsonValue:
    try:
        value = answer(proxy_config, input_value, call)
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"failed": True}
    return value


def run(input_value: Input) -> JsonValue:
    prepare_environment(input_value)
    with fake_proxy(input_value["master_key"], input_value["general_settings"]):
        proxy_config = ProxyConfig()
        return [call_answer(proxy_config, input_value, call) for call in input_value["calls"]]
