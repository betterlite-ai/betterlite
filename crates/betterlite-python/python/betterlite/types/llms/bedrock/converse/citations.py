from typing_extensions import TypedDict


class CitationWebLocationBlock(TypedDict, total=False):
    url: str
    domain: str


class CitationSearchResultLocationBlock(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    start: int
    end: int
    searchResultIndex: int


class CitationLocationBlock(TypedDict, total=False):
    web: CitationWebLocationBlock
    searchResultLocation: CitationSearchResultLocationBlock  # ast-grep-ignore: no-long-names


class CitationReferenceBlock(TypedDict, total=False):
    location: CitationLocationBlock
    source: str
    title: str


class CitationGeneratedContentBlock(TypedDict, total=False):
    text: str


class CitationsContentBlock(TypedDict, total=False):
    content: list[CitationGeneratedContentBlock]
    citations: list[CitationReferenceBlock]
