class OneLogin_Saml2_IdPMetadataParser:
    @classmethod
    def parse(cls, xml: str, entity_id: str | None = ...) -> dict[str, object]: ...
    @staticmethod
    def merge_settings(
        settings: dict[str, object],
        new_metadata_settings: dict[str, object],
    ) -> dict[str, object]: ...
