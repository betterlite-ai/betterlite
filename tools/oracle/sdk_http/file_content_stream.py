from typing import ClassVar

from not_built import Unbuilt


class FileContentStreamRequest(Unbuilt):
    task: ClassVar[str] = "B1030"

    file_content_stream: None
