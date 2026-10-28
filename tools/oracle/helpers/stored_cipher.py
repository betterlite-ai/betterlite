import logging
import os
import sys
from collections.abc import Callable, Generator
from contextlib import contextmanager
from types import ModuleType
from typing import TypeAlias, cast, override

import nacl.secret
from cryptography.exceptions import InvalidTag
from litellm.litellm_core_utils.app_crypto import AppCrypto
from litellm.proxy.common_utils import callback_utils
from litellm.proxy.common_utils import encrypt_decrypt_utils as codec
from litellm.proxy.spend_tracking import spend_tracking_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
DOCS_LINK = "https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key"

encrypt_value = cast(Callable[[Json, str | None], Json], codec.encrypt_value_helper)
decrypt_value = cast(Callable[[Json, str, str, bool], Json], codec.decrypt_value_helper)
decode_map = cast(Callable[..., Json], codec.decode_secret_map)
encrypt_map = cast(Callable[[Json, str | None], str], codec.encrypt_secret_map)


class LogCapture(logging.Filter):
    def __init__(self, field: str) -> None:
        super().__init__()
        self.field = field
        self.records: list[Json] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        message = record.getMessage()
        self.records.append(
            {
                "level": record.levelname.lower(),
                "names_field": bool(self.field) and self.field in message,
                "salt_advice": DOCS_LINK in message,
            }
        )
        return False


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


def call_codec(step: dict[str, Json]) -> Json:
    match text(step, "kind"):
        case "encrypt_value":
            result = encrypt_value(step.get("value"), optional_text(step, "new_key"))
        case "decrypt_value":
            keep = step.get("keep_original") is True
            report = text(step, "report")
            result = decrypt_value(step.get("value"), text(step, "field"), report, keep)
        case "decrypt_with":
            signing_key = text(step, "signing_key")
            result = codec.decrypt_if_encrypted_with(text(step, "value"), signing_key)
        case "encrypt_bearer":
            result = codec.encrypt_bearer_token(text(step, "value"), text(step, "prefix"))
        case "decrypt_bearer":
            result = codec.decrypt_bearer_token(text(step, "value"), text(step, "prefix"))
        case "encrypt_map":
            result = encrypt_map(step.get("value"), optional_text(step, "new_key"))
        case "decode_map":
            result = decode_map(step.get("value"), key=text(step, "field"))
        case other:
            raise ValueError(f"unknown codec step: {other}")
    return result


def run_codec_step(step: Json) -> Json:
    if not isinstance(step, dict):
        raise ValueError("each step must be an object")
    capture = LogCapture(optional_text(step, "field") or "")
    logger = logging.getLogger("LiteLLM Proxy")
    logger.filters.insert(0, capture)
    try:
        result: dict[str, Json] = {"value": call_codec(step)}
    except (ValueError, AttributeError, TypeError, codec.SecretMapDecodeError) as error:
        result = {"error": {"class": type(error).__name__, "message": str(error)}}
    finally:
        logger.removeFilter(capture)
    result["logs"] = capture.records
    return result


@contextmanager
def restored_globals(
    salt_key: str | None,
    master_key: str | None,
    general_settings: Json,
    nonce_hex: str,
) -> Generator[None]:
    fixed = FixedRandom(bytes.fromhex(nonce_hex))
    # nacl.secret binds random at import, so the box only sees a replacement in its own namespace
    box_namespace = cast(dict[str, object], vars(nacl.secret))
    saved_box_random = box_namespace["random"]
    saved_urandom = os.urandom
    saved_salt = os.environ.pop("LITELLM_SALT_KEY", None)
    logger = logging.getLogger("LiteLLM Proxy")
    saved_level = logger.level
    if salt_key is not None:
        os.environ["LITELLM_SALT_KEY"] = salt_key
    # the codec reads only these two globals, and the real module adds seconds to every helper call
    proxy_server = ModuleType("litellm.proxy.proxy_server")
    proxy_namespace = cast(dict[str, Json], vars(proxy_server))
    proxy_namespace["master_key"] = master_key
    if isinstance(general_settings, dict):
        # a missing global makes LiteLLM's import fail, its "settings unavailable" path
        proxy_namespace["general_settings"] = general_settings
    saved_proxy_server = sys.modules.get(proxy_server.__name__)
    sys.modules[proxy_server.__name__] = proxy_server
    os.urandom = box_namespace["random"] = fixed
    logger.setLevel(logging.DEBUG)
    try:
        yield
    finally:
        logger.setLevel(saved_level)
        if saved_proxy_server is None:
            del sys.modules[proxy_server.__name__]
        else:
            sys.modules[proxy_server.__name__] = saved_proxy_server
        os.urandom, box_namespace["random"] = saved_urandom, saved_box_random
        os.environ.pop("LITELLM_SALT_KEY", None)
        if saved_salt is not None:
            os.environ["LITELLM_SALT_KEY"] = saved_salt


def run_codec(input_value: dict[str, Json]) -> Json:
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    with restored_globals(
        optional_text(input_value, "salt_key"),
        optional_text(input_value, "master_key"),
        input_value.get("general_settings"),
        text(input_value, "nonce_hex"),
    ):
        return [run_codec_step(step) for step in steps]


def app_aad(step: dict[str, Json], name: str) -> bytes | None:
    value = optional_text(step, name)
    return value.encode("utf-8") if value is not None else None


def app_encrypt(app: AppCrypto, data: dict[str, Json], aad: bytes | None) -> dict[str, Json]:
    encrypt = cast(Callable[[dict[str, Json], bytes | None], dict[str, Json]], app.encrypt_json)
    return encrypt(data, aad)


def app_decrypt(app: AppCrypto, envelope: dict[str, Json], aad: bytes | None) -> Json:
    return cast(Callable[[dict[str, Json], bytes | None], Json], app.decrypt_json)(envelope, aad)


def call_app_crypto(app: AppCrypto, step: dict[str, Json]) -> Json:
    match text(step, "kind"):
        case "roundtrip":
            aad = app_aad(step, "aad")
            envelope = app_encrypt(app, cast(dict[str, Json], step.get("data")), aad)
            return app_decrypt(app, envelope, aad)
        case "aad_mismatch":
            data = cast(dict[str, Json], step.get("data"))
            envelope = app_encrypt(app, data, app_aad(step, "encrypt_aad"))
            return app_decrypt(app, envelope, app_aad(step, "decrypt_aad"))
        case "decrypt":
            envelope_value = step.get("envelope")
            if not isinstance(envelope_value, dict):
                raise ValueError("envelope must be an object")
            return app_decrypt(app, cast(dict[str, Json], envelope_value), app_aad(step, "aad"))
        case other:
            raise ValueError(f"unknown app_crypto step: {other}")


def run_app_crypto(input_value: dict[str, Json]) -> Json:
    app = AppCrypto(text(input_value, "key").encode("utf-8"))
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    results: list[Json] = []
    for step in steps:
        if not isinstance(step, dict):
            raise ValueError("each step must be an object")
        try:
            results.append({"value": call_app_crypto(app, step)})
        except (ValueError, KeyError, TypeError, InvalidTag) as error:
            results.append({"error": {"class": "ValueError", "message": str(error)}})
    return results


def encrypt_callback_var(key: str, value: object, extra: set[str]) -> object:
    if not isinstance(value, str) or not value:
        return value
    if not callback_utils.is_sensitive_callback_key(key, extra):
        return value
    if value.startswith(callback_utils.CALLBACK_VAR_ENCRYPTED_PREFIX):
        return value
    try:
        return callback_utils.CALLBACK_VAR_ENCRYPTED_PREFIX + codec.encrypt_value_helper(value)
    except Exception:  # noqa: BLE001 - no salt/master key leaves the value as-is, matching the source
        return value


def decrypt_callback_var(key: str, value: object) -> object:
    if not isinstance(value, str) or not value:
        return value
    if not value.startswith(callback_utils.CALLBACK_VAR_ENCRYPTED_PREFIX):
        return value
    inner = value[len(callback_utils.CALLBACK_VAR_ENCRYPTED_PREFIX) :]
    decrypted = codec.decrypt_value_helper(
        value=inner, key=key, exception_type="debug", return_original_value=False
    )
    return decrypted if decrypted is not None else value


def call_callback(step: dict[str, Json], extra: set[str]) -> Json:
    metadata = step.get("metadata")
    match text(step, "kind"):
        case "encrypt":
            return cast(
                Json,
                callback_utils._transform_callback_vars(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    metadata, lambda key, value: encrypt_callback_var(key, value, extra)
                ),
            )
        case "decrypt":
            return cast(
                Json,
                callback_utils._transform_callback_vars(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                    metadata, decrypt_callback_var
                ),
            )
        case other:
            raise ValueError(f"unknown callback_vars step: {other}")


def run_callback_vars(input_value: dict[str, Json]) -> Json:
    steps = input_value["steps"]
    if not isinstance(steps, list):
        raise ValueError("steps must be a list")
    extra_value = input_value.get("extra")
    if not isinstance(extra_value, list) or not all(isinstance(item, str) for item in extra_value):
        raise ValueError("extra must be a list of strings")
    extra = set(cast(list[str], extra_value))
    with restored_globals(
        optional_text(input_value, "salt_key"),
        optional_text(input_value, "master_key"),
        input_value.get("general_settings"),
        text(input_value, "nonce_hex"),
    ):
        return [call_callback(step, extra) for step in steps if isinstance(step, dict)]


def run_master_key_match(input_value: dict[str, Json]) -> Json:
    presented = optional_text(input_value, "presented")
    master_key = optional_text(input_value, "master_key")
    try:
        return spend_tracking_utils._is_master_key(presented, master_key)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except TypeError:
        return False


def run(input_value: Json) -> Json:
    if not isinstance(input_value, dict):
        raise ValueError("input must be an object")
    match text(input_value, "op"):
        case "codec":
            return run_codec(input_value)
        case "app_crypto":
            return run_app_crypto(input_value)
        case "callback_vars":
            return run_callback_vars(input_value)
        case "master_key_match":
            return run_master_key_match(input_value)
        case other:
            raise ValueError(f"unknown stored_cipher op: {other}")
