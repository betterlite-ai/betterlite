class OneLogin_Saml2_Utils:
    @staticmethod
    def add_sign(
        xml: str,
        key: str,
        cert: str,
        debug: bool = ...,
        sign_algorithm: str = ...,
        digest_algorithm: str = ...,
    ) -> str | bytes: ...
    @staticmethod
    def generate_unique_id() -> str: ...
