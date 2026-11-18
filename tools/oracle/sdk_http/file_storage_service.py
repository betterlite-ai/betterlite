from typing import ClassVar

from not_built import Unbuilt


class FileStorageServiceRequest(Unbuilt):
    task: ClassVar[str] = "B1027"

    file_storage_service: None
