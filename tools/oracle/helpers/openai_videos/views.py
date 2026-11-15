from litellm.types.videos.main import VideoObject
from pydantic import JsonValue

from .config import Parts
from .types import VIDEO_VIEW, Reference, VideoListView, VideoView


def files_json(files: Parts, reference: Reference | None) -> list[JsonValue]:
    if reference is None:
        return []
    match files:
        case [(field, (filename, _content, content_type)), *_]:
            part: dict[str, JsonValue] = {
                "field": field,
                "filename": filename,
                "content_type": content_type,
            }
            match reference["kind"]:
                case "text":
                    part["body_kind"] = "text"
                    part["body"] = reference["text"]
                case "bytes":
                    part["body_kind"] = "bytes"
                    part["body"] = reference["data_base64"]
            return [part]
        case _:
            return []


def video_json(view: VideoView) -> JsonValue:
    rendered: dict[str, JsonValue] = {
        "id": view["id"],
        "object": view["object"],
        "status": view["status"],
        "created_at": view["created_at"],
        "completed_at": view["completed_at"],
        "expires_at": view["expires_at"],
        "error": view["error"],
        "progress": view["progress"],
        "remixed_from_video_id": view["remixed_from_video_id"],
        "seconds": view["seconds"],
        "size": view["size"],
        "model": view["model"],
        "usage": view["usage"],
    }
    return rendered


def video_view(video: VideoObject) -> JsonValue:
    view = VIDEO_VIEW.validate_json(video.model_dump_json())
    return video_json(view)


def list_view(reply: VideoListView) -> JsonValue:
    data: list[JsonValue] = [video_json(video) for video in reply["data"]]
    rendered: dict[str, JsonValue] = {
        "data": data,
        "first_id": reply["first_id"],
        "last_id": reply["last_id"],
        "object": reply["object"],
        "has_more": reply["has_more"],
    }
    return rendered


def request_view(url: str, params: dict[str, JsonValue]) -> JsonValue:
    rendered: dict[str, JsonValue] = {"url": url, "params": params}
    return rendered
