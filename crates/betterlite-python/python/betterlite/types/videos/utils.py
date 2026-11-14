import base64
import binascii
import logging
from dataclasses import dataclass
from typing import Final

from betterlite.types.videos.main import DecodedVideoId

LOGGER: Final = logging.getLogger("LiteLLM")

VIDEO_ID_PREFIX: Final = "video_"
CHARACTER_ID_PREFIX: Final = "character_"
_VIDEO_ID_TEMPLATE: Final = "litellm:custom_llm_provider:{};model_id:{};video_id:{}"
CHARACTER_ID_TEMPLATE: Final = "litellm:custom_llm_provider:{};model_id:{};character_id:{}"

_PROVIDER_PREFIX: Final = "litellm:custom_llm_provider:"
_MODEL_ID_PREFIX: Final = "model_id:"
_BASE64_QUANTUM: Final = 4
_MIN_ID_PARTS: Final = 3


@dataclass(frozen=True, slots=True)
class _IdKind:
    label: str
    prefix: str
    template: str
    field_prefix: str


_VIDEO_KIND: Final = _IdKind("video_id", VIDEO_ID_PREFIX, _VIDEO_ID_TEMPLATE, "video_id:")
_CHARACTER_KIND: Final = _IdKind(
    "character_id", CHARACTER_ID_PREFIX, CHARACTER_ID_TEMPLATE, "character_id:",
)


class DecodedCharacterId(dict[str, str | None]):
    custom_llm_provider: str | None
    model_id: str | None
    character_id: str


def _add_base64_padding(value: str) -> str:
    missing_padding: Final = len(value) % _BASE64_QUANTUM
    if missing_padding:
        return value + "=" * (_BASE64_QUANTUM - missing_padding)
    return value


def _decode_id_parts(encoded_id: str, kind: _IdKind) -> tuple[str | None, str | None, str]:
    if not encoded_id or not encoded_id.startswith(kind.prefix):
        return None, None, encoded_id
    try:
        cleaned_id: Final = _add_base64_padding(encoded_id.replace(kind.prefix, ""))
        decoded_id: Final = base64.b64decode(cleaned_id.encode("utf-8")).decode("utf-8")
        if ";" not in decoded_id:
            return None, None, encoded_id
        parts: Final = decoded_id.split(";")
        if len(parts) < _MIN_ID_PARTS:
            return None, None, encoded_id
        return (
            parts[0].replace(_PROVIDER_PREFIX, ""),
            parts[1].replace(_MODEL_ID_PREFIX, ""),
            parts[2].replace(kind.field_prefix, ""),
        )
    except (binascii.Error, UnicodeDecodeError) as error:
        LOGGER.debug("Error decoding %s '%s': %s", kind.label, encoded_id, error)
        return None, None, encoded_id


def _is_encodable(value: str, provider: str) -> bool:
    if not provider:
        return False
    return bool(value)


def _encode_id_with_provider(
    value: str, provider: str, model_id: str | None, kind: _IdKind,
) -> str:
    already_encoded: Final = _decode_id_parts(value, kind)
    if already_encoded[0] is not None:
        return value
    selected_model_id: Final = "" if model_id is None else model_id
    assembled_id: Final = kind.template.format(provider, selected_model_id, value)
    return f"{kind.prefix}{base64.b64encode(assembled_id.encode('utf-8')).decode('utf-8')}"


def encode_video_id_with_provider(
    video_id: str, provider: str, model_id: str | None = None,
) -> str:
    if not _is_encodable(video_id, provider):
        return video_id
    return _encode_id_with_provider(video_id, provider, model_id, _VIDEO_KIND)


def decode_video_id_with_provider(encoded_video_id: str) -> DecodedVideoId:
    decoded: Final = _decode_id_parts(encoded_video_id, _VIDEO_KIND)
    return DecodedVideoId(
        custom_llm_provider=decoded[0], model_id=decoded[1], video_id=decoded[2],
    )


def extract_original_video_id(encoded_video_id: str) -> str:
    decoded: Final = decode_video_id_with_provider(encoded_video_id)
    return decoded["video_id"]


# ast-grep-ignore: no-long-names
def encode_character_id_with_provider(
    character_id: str, provider: str, model_id: str | None = None,
) -> str:
    if not _is_encodable(character_id, provider):
        return character_id
    return _encode_id_with_provider(character_id, provider, model_id, _CHARACTER_KIND)


# ast-grep-ignore: no-long-names
def decode_character_id_with_provider(encoded_character_id: str) -> DecodedCharacterId:
    decoded: Final = _decode_id_parts(encoded_character_id, _CHARACTER_KIND)
    return DecodedCharacterId(
        custom_llm_provider=decoded[0], model_id=decoded[1], character_id=decoded[2],
    )


def extract_original_character_id(encoded_character_id: str) -> str:
    decoded: Final = decode_character_id_with_provider(encoded_character_id)
    found: Final = decoded["character_id"]
    return encoded_character_id if found is None else found
