import io
from dataclasses import dataclass, field
from typing import Final, Literal

import litellm
from openai.types.audio.transcription_create_params import FileTypes
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

litellm.suppress_debug_info = True

NOW: Final = 1_700_000_000


@dataclass
class Reply:
    status: int = 200
    headers: dict[str, str] = field(default_factory=dict)
    body: str = ""


@dataclass
class Probe:
    model: str | None = None
    image_filename: str | None = None
    image_content: str = ""
    image_content_type: str | None = None
    optional_params: dict[str, Json] = field(default_factory=dict)
    non_default_params: dict[str, Json] = field(default_factory=dict)
    existing_params: dict[str, Json] = field(default_factory=dict)
    async_call: bool = False
    api_key: str | None = None
    api_base: str | None = None
    organization: str | None = None
    timeout: int | None = None
    client: Literal["sync", "async"] | None = None
    reply: Reply = field(default_factory=Reply)
    failure: Literal["timeout", "failed"] | None = None
    missing_config: bool = False


PROBE: Final = TypeAdapter(Probe)
DOCUMENT: Final = TypeAdapter(dict[str, Json])


class NamedBytesIO(io.BytesIO):
    name: str

    def __init__(self, content: bytes, name: str) -> None:
        super().__init__(content)
        self.name = name


def image_view(probe: Probe) -> Json:
    return {
        "filename": probe.image_filename,
        "content_type": probe.image_content_type,
        "size": len(probe.image_content.encode()),
    }


def file_object(probe: Probe) -> FileTypes:
    content = probe.image_content.encode()
    if probe.image_content_type is not None:
        return (probe.image_filename or "upload", content, probe.image_content_type)
    if probe.image_filename is not None:
        return NamedBytesIO(content, probe.image_filename)
    return content


def data_view(data: dict[str, Json], probe: Probe) -> Json:
    return {name: value for name, value in data.items() if name != "image"} | {"image": image_view(probe)}
