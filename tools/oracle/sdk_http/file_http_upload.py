from typing import ClassVar

from not_built import Unbuilt


class FileHttpUploadRequest(Unbuilt):
    task: ClassVar[str] = "B2461"

    file_http_upload: None
