from pydantic import RootModel


class ModelOption(RootModel[str]):
    root: str


class AgentOption(RootModel[str]):
    root: str


class ImageMimeTypeOption(RootModel[str]):
    root: str


class AudioMimeTypeOption(RootModel[str]):
    root: str


class VideoMimeTypeOption(RootModel[str]):
    root: str
