from typing import ClassVar

from not_built import Unbuilt


class FileUploadRoutingRequest(Unbuilt):
    task: ClassVar[str] = "B1031"

    file_upload_routing: None
