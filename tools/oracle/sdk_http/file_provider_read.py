from typing import ClassVar

from not_built import Unbuilt


class FileProviderReadRequest(Unbuilt):
    task: ClassVar[str] = "B1029"

    file_provider_read: None
