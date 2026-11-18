from typing import ClassVar

from not_built import Unbuilt


class FileHttpMetadataRequest(Unbuilt):
    task: ClassVar[str] = "B2462"

    file_http_metadata: None
