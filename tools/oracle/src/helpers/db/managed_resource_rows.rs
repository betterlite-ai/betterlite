use betterlite_db::managed_resource_rows::{
    ManagedResourceRow, ManagedRowDependencies, ManagedRowsDecodeError,
    decode_managed_resource_rows,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::files::records::FileObject;
use betterlite_openai::fine_tuning::job::FineTuneJob;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

type Rows = ManagedResourceRow<FileObject, BatchObject, FineTuneJob, ResponsesApiResponse>;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    salt_key: Option<String>,
    rows: Vec<RowCall>,
}

#[derive(Deserialize)]
struct RowCall {
    name: String,
    stored: String,
}

#[derive(Serialize)]
struct RowResult<'name> {
    name: &'name str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    row: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseline_model: Option<String>,
}

fn helper_error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn cipher(salt_key: Option<&String>) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: salt_key.map(|key| SecretString::from(key.clone())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn row_result(
    call: &RowCall,
    result: Result<Rows, ManagedRowsDecodeError>,
) -> Result<RowResult<'_>, Box<crate::helpers::HelperError>> {
    let Ok(decoded) = result else {
        return Ok(RowResult {
            name: &call.name,
            ok: false,
            row: None,
            baseline_model: None,
        });
    };
    let baseline_model = match &decoded {
        ManagedResourceRow::AutoRouterSession(session) => {
            session.baseline_model().map(str::to_owned)
        }
        ManagedResourceRow::ManagedFile(_)
        | ManagedResourceRow::ManagedObject(_)
        | ManagedResourceRow::ManagedVectorStore(_)
        | ManagedResourceRow::ManagedVectorStores(_)
        | ManagedResourceRow::McpEnvVar(_)
        | ManagedResourceRow::McpServer(_)
        | ManagedResourceRow::Skill(_)
        | ManagedResourceRow::SpendLog(_)
        | ManagedResourceRow::ErrorLog(_)
        | ManagedResourceRow::Tag(_) => None,
    };
    let row = serde_json::to_value(&decoded)
        .map_err(|source| helper_error("InvalidOutput", &source.to_string()))?;
    Ok(RowResult {
        name: &call.name,
        ok: true,
        row: Some(row),
        baseline_model,
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let cipher = cipher(input.salt_key.as_ref());
        let dependencies = ManagedRowDependencies {
            cipher: &cipher,
            error_request_id: Uuid::nil(),
        };
        let rows = input
            .rows
            .iter()
            .map(|call| {
                let decoded = decode_managed_resource_rows::<
                    FileObject,
                    BatchObject,
                    FineTuneJob,
                    ResponsesApiResponse,
                >(call.stored.as_bytes(), &dependencies);
                row_result(call, decoded)
            })
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_value(json!({ "rows": rows }))
            .map_err(|source| helper_error("InvalidOutput", &source.to_string()))
    }
}
