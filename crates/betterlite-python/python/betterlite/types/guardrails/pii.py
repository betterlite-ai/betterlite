from enum import StrEnum
from typing import Final, Literal, TypedDict

from pydantic import Field, JsonValue

from betterlite.types.utils.base import ResponseObject


class PiiAction(StrEnum):
    BLOCK = "BLOCK"
    MASK = "MASK"


class PiiEntityCategory(StrEnum):
    GENERAL = "General"
    FINANCE = "Finance"
    USA = "USA"
    UK = "UK"
    SPAIN = "Spain"
    ITALY = "Italy"
    POLAND = "Poland"
    SINGAPORE = "Singapore"
    AUSTRALIA = "Australia"
    INDIA = "India"
    FINLAND = "Finland"
    GERMANY = "Germany"
    KOREA = "Korea"
    CANADA = "Canada"
    SWEDEN = "Sweden"
    THAILAND = "Thailand"
    TURKEY = "Turkey"
    NIGERIA = "Nigeria"
    PHILIPPINES = "Philippines"
    SOUTH_AFRICA = "South Africa"


class PiiEntityType(StrEnum):
    CREDIT_CARD = "CREDIT_CARD"
    CRYPTO = "CRYPTO"
    DATE_TIME = "DATE_TIME"
    EMAIL_ADDRESS = "EMAIL_ADDRESS"
    IBAN_CODE = "IBAN_CODE"
    IP_ADDRESS = "IP_ADDRESS"
    NRP = "NRP"
    LOCATION = "LOCATION"
    PERSON = "PERSON"
    PHONE_NUMBER = "PHONE_NUMBER"
    MEDICAL_LICENSE = "MEDICAL_LICENSE"
    URL = "URL"
    MAC_ADDRESS = "MAC_ADDRESS"
    UUID = "UUID"
    US_BANK_NUMBER = "US_BANK_NUMBER"
    US_DRIVER_LICENSE = "US_DRIVER_LICENSE"
    US_ITIN = "US_ITIN"
    US_PASSPORT = "US_PASSPORT"
    US_SSN = "US_SSN"
    US_MBI = "US_MBI"
    US_NPI = "US_NPI"
    UK_NHS = "UK_NHS"
    UK_NINO = "UK_NINO"
    UK_PASSPORT = "UK_PASSPORT"
    UK_POSTCODE = "UK_POSTCODE"
    UK_VEHICLE_REGISTRATION = "UK_VEHICLE_REGISTRATION"
    UK_DRIVING_LICENCE = "UK_DRIVING_LICENCE"
    ES_NIF = "ES_NIF"
    ES_NIE = "ES_NIE"
    ES_PASSPORT = "ES_PASSPORT"
    IT_FISCAL_CODE = "IT_FISCAL_CODE"
    IT_DRIVER_LICENSE = "IT_DRIVER_LICENSE"
    IT_VAT_CODE = "IT_VAT_CODE"
    IT_PASSPORT = "IT_PASSPORT"
    IT_IDENTITY_CARD = "IT_IDENTITY_CARD"
    PL_PESEL = "PL_PESEL"
    SG_NRIC_FIN = "SG_NRIC_FIN"
    SG_UEN = "SG_UEN"
    AU_ABN = "AU_ABN"
    AU_ACN = "AU_ACN"
    AU_TFN = "AU_TFN"
    AU_MEDICARE = "AU_MEDICARE"
    IN_PAN = "IN_PAN"
    IN_AADHAAR = "IN_AADHAAR"
    IN_VEHICLE_REGISTRATION = "IN_VEHICLE_REGISTRATION"
    IN_VOTER = "IN_VOTER"
    IN_PASSPORT = "IN_PASSPORT"
    IN_GSTIN = "IN_GSTIN"
    FI_PERSONAL_IDENTITY_CODE = "FI_PERSONAL_IDENTITY_CODE"
    DE_TAX_ID = "DE_TAX_ID"
    DE_TAX_NUMBER = "DE_TAX_NUMBER"
    DE_VAT_ID = "DE_VAT_ID"
    DE_PASSPORT = "DE_PASSPORT"
    DE_ID_CARD = "DE_ID_CARD"
    DE_FUEHRERSCHEIN = "DE_FUEHRERSCHEIN"
    DE_SOCIAL_SECURITY = "DE_SOCIAL_SECURITY"
    DE_HEALTH_INSURANCE = "DE_HEALTH_INSURANCE"
    DE_LANR = "DE_LANR"
    DE_BSNR = "DE_BSNR"
    DE_KFZ = "DE_KFZ"
    DE_HANDELSREGISTER = "DE_HANDELSREGISTER"
    DE_PLZ = "DE_PLZ"
    KR_RRN = "KR_RRN"
    KR_FRN = "KR_FRN"
    KR_PASSPORT = "KR_PASSPORT"
    KR_DRIVER_LICENSE = "KR_DRIVER_LICENSE"
    KR_BRN = "KR_BRN"
    CA_SIN = "CA_SIN"
    SE_PERSONNUMMER = "SE_PERSONNUMMER"
    SE_ORGANISATIONSNUMMER = "SE_ORGANISATIONSNUMMER"
    TH_TNIN = "TH_TNIN"
    TR_NATIONAL_ID = "TR_NATIONAL_ID"
    TR_LICENSE_PLATE = "TR_LICENSE_PLATE"
    NG_NIN = "NG_NIN"
    NG_VEHICLE_REGISTRATION = "NG_VEHICLE_REGISTRATION"
    PH_TIN = "PH_TIN"
    PH_UMID = "PH_UMID"
    PH_PASSPORT = "PH_PASSPORT"
    ZA_ID_NUMBER = "ZA_ID_NUMBER"


PII_ENTITY_CATEGORIES_MAP: Final[dict[PiiEntityCategory, tuple[PiiEntityType, ...]]] = {
    PiiEntityCategory.GENERAL: (
        PiiEntityType.DATE_TIME, PiiEntityType.EMAIL_ADDRESS, PiiEntityType.IP_ADDRESS,
        PiiEntityType.NRP, PiiEntityType.LOCATION, PiiEntityType.PERSON,
        PiiEntityType.PHONE_NUMBER, PiiEntityType.MEDICAL_LICENSE, PiiEntityType.URL,
        PiiEntityType.MAC_ADDRESS, PiiEntityType.UUID,
    ),
    PiiEntityCategory.FINANCE: (
        PiiEntityType.CREDIT_CARD, PiiEntityType.CRYPTO, PiiEntityType.IBAN_CODE,
    ),
    PiiEntityCategory.USA: (
        PiiEntityType.US_BANK_NUMBER, PiiEntityType.US_DRIVER_LICENSE,
        PiiEntityType.US_ITIN, PiiEntityType.US_PASSPORT, PiiEntityType.US_SSN,
        PiiEntityType.US_MBI, PiiEntityType.US_NPI,
    ),
    PiiEntityCategory.UK: (
        PiiEntityType.UK_NHS, PiiEntityType.UK_NINO, PiiEntityType.UK_PASSPORT,
        PiiEntityType.UK_POSTCODE, PiiEntityType.UK_VEHICLE_REGISTRATION,
        PiiEntityType.UK_DRIVING_LICENCE,
    ),
    PiiEntityCategory.SPAIN: (
        PiiEntityType.ES_NIF, PiiEntityType.ES_NIE, PiiEntityType.ES_PASSPORT,
    ),
    PiiEntityCategory.ITALY: (
        PiiEntityType.IT_FISCAL_CODE, PiiEntityType.IT_DRIVER_LICENSE,
        PiiEntityType.IT_VAT_CODE, PiiEntityType.IT_PASSPORT,
        PiiEntityType.IT_IDENTITY_CARD,
    ),
    PiiEntityCategory.POLAND: (PiiEntityType.PL_PESEL,),
    PiiEntityCategory.SINGAPORE: (PiiEntityType.SG_NRIC_FIN, PiiEntityType.SG_UEN),
    PiiEntityCategory.AUSTRALIA: (
        PiiEntityType.AU_ABN, PiiEntityType.AU_ACN, PiiEntityType.AU_TFN,
        PiiEntityType.AU_MEDICARE,
    ),
    PiiEntityCategory.INDIA: (
        PiiEntityType.IN_PAN, PiiEntityType.IN_AADHAAR,
        PiiEntityType.IN_VEHICLE_REGISTRATION, PiiEntityType.IN_VOTER,
        PiiEntityType.IN_PASSPORT, PiiEntityType.IN_GSTIN,
    ),
    PiiEntityCategory.FINLAND: (PiiEntityType.FI_PERSONAL_IDENTITY_CODE,),
    PiiEntityCategory.GERMANY: (
        PiiEntityType.DE_TAX_ID, PiiEntityType.DE_TAX_NUMBER, PiiEntityType.DE_VAT_ID,
        PiiEntityType.DE_PASSPORT, PiiEntityType.DE_ID_CARD,
        PiiEntityType.DE_FUEHRERSCHEIN, PiiEntityType.DE_SOCIAL_SECURITY,
        PiiEntityType.DE_HEALTH_INSURANCE, PiiEntityType.DE_LANR, PiiEntityType.DE_BSNR,
        PiiEntityType.DE_KFZ, PiiEntityType.DE_HANDELSREGISTER, PiiEntityType.DE_PLZ,
    ),
    PiiEntityCategory.KOREA: (
        PiiEntityType.KR_RRN, PiiEntityType.KR_FRN, PiiEntityType.KR_PASSPORT,
        PiiEntityType.KR_DRIVER_LICENSE, PiiEntityType.KR_BRN,
    ),
    PiiEntityCategory.CANADA: (PiiEntityType.CA_SIN,),
    PiiEntityCategory.SWEDEN: (
        PiiEntityType.SE_PERSONNUMMER, PiiEntityType.SE_ORGANISATIONSNUMMER,
    ),
    PiiEntityCategory.THAILAND: (PiiEntityType.TH_TNIN,),
    PiiEntityCategory.TURKEY: (
        PiiEntityType.TR_NATIONAL_ID, PiiEntityType.TR_LICENSE_PLATE,
    ),
    PiiEntityCategory.NIGERIA: (
        PiiEntityType.NG_NIN, PiiEntityType.NG_VEHICLE_REGISTRATION,
    ),
    PiiEntityCategory.PHILIPPINES: (
        PiiEntityType.PH_TIN, PiiEntityType.PH_UMID, PiiEntityType.PH_PASSPORT,
    ),
    PiiEntityCategory.SOUTH_AFRICA: (PiiEntityType.ZA_ID_NUMBER,),
}


class PiiEntityCategoryMap(TypedDict):
    category: str
    entities: list[str]


class GuardrailParamUITypes(StrEnum):
    BOOL = "bool"
    STR = "str"
    MULTISELECT = "multiselect"
    PERCENTAGE = "percentage"


class PresidioPresidioConfigModelUserInterface(ResponseObject):  # ast-grep-ignore: no-long-names
    presidio_analyzer_api_base: str | None = None
    presidio_anonymizer_api_base: str | None = None
    presidio_filter_scope: Literal["input", "output", "both"] | None = None
    output_parse_pii: bool | None = Field(
        default=None, json_schema_extra={"ui_type": GuardrailParamUITypes.BOOL.value}
    )
    presidio_language: str | None = "en"
    presidio_run_on: Literal["input", "output", "both"] | None = None


# ast-grep-ignore: no-long-names
class PresidioConfigModel(PresidioPresidioConfigModelUserInterface):
    pii_entities_config: dict[PiiEntityType | str, PiiAction] | None = None
    presidio_score_thresholds: dict[PiiEntityType | str, float] | None = None
    presidio_entities_deny_list: list[PiiEntityType | str] | None = None
    presidio_ad_hoc_recognizers: str | None = None
    # ast-grep-ignore: no-long-names
    presidio_analyze_chunk_size_bytes: int | None = None
    mock_redacted_text: dict[str, JsonValue] | None = None
