import base64
from typing import Literal

import litellm
from openai_videos import CONFIG as VIDEO_CONFIG, create_ops, reply_ops, request_ops
from openai_videos.views import (
    BytesReference,
    ContentReplyCall,
    ContentRequestCall,
    CreateCall,
    CreateReplyCall,
    DeleteReplyCall,
    DeleteRequestCall,
    ListReplyCall,
    ListRequestCall,
    Reference,
    RemixReplyCall,
    RemixRequestCall,
    StatusReplyCall,
    StatusRequestCall,
    VideoReplyOps,
    VideoRequestOps,
)
from pydantic import JsonValue

from .cases import VideoCases, VideoCreateCase, VideoLifecycleCase
from .support import MODEL, OBJECT, OBJECTS, TEXT, leaf_parts, required

litellm.suppress_debug_info = True


def video_create(case: VideoCreateCase) -> JsonValue:
    base = TEXT.validate_python(VIDEO_CONFIG.get_complete_url(case["model"], case["api_base"], {}))
    reference: Reference | None = None
    given = case["reference"]
    if given is not None:
        reference = BytesReference(
            kind="bytes", data_base64=base64.b64encode(given.encode()).decode("ascii")
        )
    create_call: CreateCall = {
        "op": "create",
        "model": case["model"],
        "prompt": case["prompt"],
        "api_base": base,
        "options": case["options"],
        "reference": reference,
    }
    created = OBJECT.validate_python(create_ops(create_call))
    create_reply: CreateReplyCall = {
        "op": "create_response",
        "model": case["model"],
        "provider": case["provider"],
        "body_base64": base64.b64encode(case["reply"].encode()).decode("ascii"),
    }
    reply = create_ops(create_reply)
    parts = leaf_parts(OBJECTS.validate_python(created["files"]))
    return {
        "url": created["api_base"],
        "body": created["data"],
        "parts": parts,
        "multipart": created["multipart"],
        "reply": reply,
    }


def video_cases(case: VideoCases) -> JsonValue:
    match case["case"]:
        case "video_create":
            return video_create(case)
        case "video_lifecycle":
            return video_lifecycle(case)


def video_lifecycle(case: VideoLifecycleCase) -> JsonValue:
    base = TEXT.validate_python(VIDEO_CONFIG.get_complete_url(MODEL, case["api_base"], {}))
    match case["op"]:
        case "list":
            return lifecycle_list(case, base)
        case "status":
            return lifecycle_status(case, base)
        case "delete":
            return lifecycle_delete(case, base)
        case "content":
            return lifecycle_content(case, base)
        case "remix":
            return lifecycle_remix(case, base)


def lifecycle_list(case: VideoLifecycleCase, base: str) -> JsonValue:
    encoded = base64.b64encode(case["reply"].encode()).decode("ascii")
    call = ListRequestCall(
        op="list_request", api_base=base, after=case["after"], limit=case["limit"], order=case["order"]
    )
    reply = ListReplyCall(op="list_response", provider=case["provider"], body_base64=encoded)
    return lifecycle_answer("GET", call, reply, "query")


def lifecycle_status(case: VideoLifecycleCase, base: str) -> JsonValue:
    video_id = required(case["video_id"], "video_id")
    encoded = base64.b64encode(case["reply"].encode()).decode("ascii")
    call = StatusRequestCall(op="status_request", video_id=video_id, api_base=base)
    reply = StatusReplyCall(op="status_response", provider=case["provider"], body_base64=encoded)
    return lifecycle_answer("GET", call, reply, "none")


def lifecycle_delete(case: VideoLifecycleCase, base: str) -> JsonValue:
    video_id = required(case["video_id"], "video_id")
    encoded = base64.b64encode(case["reply"].encode()).decode("ascii")
    call = DeleteRequestCall(op="delete_request", video_id=video_id, api_base=base)
    reply = DeleteReplyCall(op="delete_response", body_base64=encoded)
    return lifecycle_answer("DELETE", call, reply, "none")


def lifecycle_content(case: VideoLifecycleCase, base: str) -> JsonValue:
    video_id = required(case["video_id"], "video_id")
    encoded = base64.b64encode(case["reply"].encode()).decode("ascii")
    call = ContentRequestCall(
        op="content_request", video_id=video_id, api_base=base, variant=case["variant"]
    )
    reply = ContentReplyCall(op="content_response", body_base64=encoded)
    return lifecycle_answer("GET", call, reply, "none")


def lifecycle_remix(case: VideoLifecycleCase, base: str) -> JsonValue:
    video_id = required(case["video_id"], "video_id")
    prompt = required(case["prompt"], "prompt")
    encoded = base64.b64encode(case["reply"].encode()).decode("ascii")
    call = RemixRequestCall(op="remix_request", video_id=video_id, prompt=prompt, api_base=base)
    reply = RemixReplyCall(op="remix_response", provider=case["provider"], body_base64=encoded)
    return lifecycle_answer("POST", call, reply, "body")


def lifecycle_answer(
    method: str,
    request: VideoRequestOps,
    reply: VideoReplyOps,
    source: Literal["query", "body", "none"],
) -> JsonValue:
    sent = OBJECT.validate_python(request_ops(request))
    params: JsonValue = sent["params"] if source == "query" else {}
    body: JsonValue = sent["params"] if source == "body" else None
    match reply["op"]:
        case "content_response":
            content = OBJECT.validate_python(reply_ops(reply))
            answer: JsonValue = {
                "body": base64.b64decode(TEXT.validate_python(content["body_base64"])).decode()
            }
        case (
            "list_response" | "status_response" | "delete_response" | "remix_response"
        ):
            answer = reply_ops(reply)
    return {
        "request": {
            "method": method,
            "url": sent["url"],
            "params": params,
            "body": body,
            "parts": [],
            "multipart": False,
        },
        "reply": answer,
    }
