import base64
from typing import Final

import litellm
from pydantic import JsonValue, TypeAdapter

from openai_video_edits import (
    CreateCharacterRequest,
    EditRequest,
    ExtensionRequest,
    GetCharacterRequest,
    create_character_reply,
    create_character_value,
    edit_reply,
    edit_value,
    extension_reply,
    extension_value,
    get_character_reply,
    get_character_value,
)
from openai_video_edits.fixtures import BytesFixture, EditBody, Fixture
from openai_videos.config import CONFIG as VIDEO_CONFIG

from .cases import VideoExtendedCase
from .support import OBJECT, OBJECTS, TEXT, leaf_parts, required

litellm.suppress_debug_info = True

EDIT_BODY: Final = TypeAdapter(EditBody)


def video_edit_cases(case: VideoExtendedCase) -> JsonValue:
    base = TEXT.validate_python(VIDEO_CONFIG.get_complete_url(case["model"], case["api_base"], {}))
    match case["op"]:
        case "edit":
            return extended_edit(case, base)
        case "extension":
            return extended_extension(case, base)
        case "create_character":
            return extended_create_character(case, base)
        case "get_character":
            return extended_get_character(case, base)


def video_fixture(given: str | None) -> Fixture | None:
    if given is None:
        return None
    return BytesFixture(kind="bytes", data_base64=base64.b64encode(given.encode()).decode("ascii"))


def extended_edit(case: VideoExtendedCase, base: str) -> JsonValue:
    extra = case["extra_body"]
    request: EditRequest = {
        "op": "edit_request",
        "prompt": required(case["prompt"], "prompt"),
        "video_id": required(case["video_id"], "video_id"),
        "api_base": base,
        "fixture": video_fixture(case["video_file"]),
        "extra_body": None if extra is None else EDIT_BODY.validate_python(extra),
        "prefetched_source_data": None,
    }
    sent = OBJECT.validate_python(edit_value(request))
    reply = edit_reply(
        {
            "op": "edit_response",
            "body": case["reply"],
            "provider": case["provider"],
            "request_data": None,
        }
    )
    files = sent["files"]
    if files is None:
        return extended_answer(sent, sent["body"], [], reply, multipart=False)
    parts = leaf_parts(OBJECTS.validate_python(files))
    return extended_answer(sent, sent["body"], parts, reply, multipart=True)


def extended_extension(case: VideoExtendedCase, base: str) -> JsonValue:
    extra = case["extra_body"]
    request: ExtensionRequest = {
        "op": "extension_request",
        "prompt": required(case["prompt"], "prompt"),
        "video_id": required(case["video_id"], "video_id"),
        "seconds": required(case["seconds"], "seconds"),
        "api_base": base,
        "extra_body": None if extra is None else EDIT_BODY.validate_python(extra),
    }
    sent = OBJECT.validate_python(extension_value(request))
    reply = extension_reply(
        {"op": "extension_response", "body": case["reply"], "provider": case["provider"]}
    )
    return extended_answer(sent, sent["body"], [], reply, multipart=False)


def extended_create_character(case: VideoExtendedCase, base: str) -> JsonValue:
    video = required(case["video"], "video")
    request: CreateCharacterRequest = {
        "op": "create_character_request",
        "name": required(case["name"], "name"),
        "fixture": BytesFixture(
            kind="bytes", data_base64=base64.b64encode(video.encode()).decode("ascii")
        ),
        "api_base": base,
    }
    sent = OBJECT.validate_python(create_character_value(request))
    reply = create_character_reply({"op": "create_character_response", "body": case["reply"]})
    parts = leaf_parts(OBJECTS.validate_python(sent["files"]))
    return extended_answer(sent, None, parts, reply, multipart=True)


def extended_get_character(case: VideoExtendedCase, base: str) -> JsonValue:
    request: GetCharacterRequest = {
        "op": "get_character_request",
        "character_id": required(case["character_id"], "character_id"),
        "api_base": base,
    }
    reply = get_character_reply({"op": "get_character_response", "body": case["reply"]})
    return extended_answer(
        OBJECT.validate_python(get_character_value(request)), None, [], reply, multipart=False
    )


def extended_answer(
    sent: dict[str, JsonValue],
    body: JsonValue,
    parts: list[JsonValue],
    reply: JsonValue,
    *,
    multipart: bool,
) -> JsonValue:
    return {
        "url": sent["url"],
        "body": body,
        "parts": parts,
        "reply": reply,
        "multipart": multipart,
    }
