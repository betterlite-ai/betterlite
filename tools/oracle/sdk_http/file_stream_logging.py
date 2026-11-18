from typing import ClassVar

from not_built import Unbuilt


class FileStreamLoggingRequest(Unbuilt):
    task: ClassVar[str] = "B1080"

    file_stream_logging: None
