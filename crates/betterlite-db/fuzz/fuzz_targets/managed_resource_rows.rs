#![no_main]

use betterlite_db::managed_resource_rows::{ManagedRowDependencies, decode_managed_resource_rows};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::files::records::FileObject;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;
use uuid::Uuid;

fuzz_target!(|data: &[u8]| {
    let cipher = StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("managed-resource-rows-fuzz")),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    };
    let dependencies = ManagedRowDependencies {
        cipher: &cipher,
        error_request_id: Uuid::nil(),
    };
    drop(decode_managed_resource_rows::<
        FileObject,
        BatchObject,
        FineTuneJob,
        ResponsesApiResponse,
    >(data, &dependencies));
});
