import os
import sys
from collections.abc import Callable, Generator
from contextlib import contextmanager
from types import ModuleType
from typing import TypeAlias, cast

import nacl.secret
from litellm.proxy.common_utils import callback_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
FIELDS = {"salt_key", "master_key", "general_settings", "nonce_hex", "extra", "steps"}

strip_callback_config = cast(Callable[[Json], Json], callback_utils.strip_callback_config)
encrypt_callback_vars = cast(Callable[[Json], Json], callback_utils.encrypt_callback_vars)
decrypt_callback_vars = cast(Callable[[Json], Json], callback_utils.decrypt_callback_vars)
is_sensitive_callback_key = cast(
    Callable[[str, set[str] | None], bool], callback_utils.is_sensitive_callback_key
)


class FixedRandom:
    def __init__(self, nonce: bytes) -> None:
        self.nonce = nonce

    def __call__(self, size: int) -> bytes:
        return self.nonce[:size]


def text(step: dict[str, Json], name: str) -> str:
    value = step.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_text(step: dict[str, Json], name: str) -> str | None:
    value = step.get(name)
    if value is not None and not isinstance(value, str):
        raise ValueError(f"{name} must be a string or null")
    return value


@contextmanager
def restored_globals(input_value: dict[str, Json]) -> Generator[None]:
    settings = input_value.get("general_settings")
    fixed = FixedRandom(bytes.fromhex(text(input_value, "nonce_hex")))
    # nacl.secret binds random at import, so the box only sees a replacement in its own namespace
    box_namespace = cast(dict[str, object], vars(nacl.secret))
    saved_box_random = box_namespace["random"]
    saved_urandom = os.urandom
    saved_salt = os.environ.pop("LITELLM_SALT_KEY", None)
    salt = optional_text(input_value, "salt_key")
    if salt is not None:
        os.environ["LITELLM_SALT_KEY"] = salt
    # the codec reads only these two globals, and the real module adds seconds to every helper call
    proxy_server = ModuleType("litellm.proxy.proxy_server")
    proxy_namespace = cast(dict[str, Json], vars(proxy_server))
    proxy_namespace["master_key"] = optional_text(input_value, "master_key")
    if isinstance(settings, dict):
        # a missing global makes LiteLLM's import fail, its "settings unavailable" path
        proxy_namespace["general_settings"] = settings
    saved_proxy_server = sys.modules.get(proxy_server.__name__)
    sys.modules[proxy_server.__name__] = proxy_server
    os.urandom = box_namespace["random"] = fixed
    try:
        yield
    finally:
        if saved_proxy_server is None:
            del sys.modules[proxy_server.__name__]
        else:
            sys.modules[proxy_server.__name__] = saved_proxy_server
        os.urandom, box_namespace["random"] = saved_urandom, saved_box_random
        os.environ.pop("LITELLM_SALT_KEY", None)
        if saved_salt is not None:
            os.environ["LITELLM_SALT_KEY"] = saved_salt


def run_step(step: Json, extra: set[str]) -> Json:
    if not isinstance(step, dict):
        raise ValueError("each step must be an object")
    match text(step, "kind"):
        case "strip":
            return strip_callback_config(step.get("metadata"))
        case "encrypt":
            return encrypt_callback_vars(step.get("metadata"))
        case "decrypt":
            return decrypt_callback_vars(step.get("metadata"))
        case "sensitive":
            return is_sensitive_callback_key(text(step, "key"), extra)
        case other:
            raise ValueError(f"unknown callback_secret_fields step: {other}")


def run(input_value: Json) -> Json:
    if not isinstance(input_value, dict) or set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    extra_value = input_value["extra"]
    if not isinstance(extra_value, list) or not all(isinstance(item, str) for item in extra_value):
        raise ValueError("extra must be a list of strings")
    extra = set(cast(list[str], extra_value))
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    with restored_globals(input_value):
        return [run_step(step, extra) for step in steps]
