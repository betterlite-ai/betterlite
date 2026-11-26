use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::database_replica::pinned::WriterPinnedClient;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::key_hash::KeyHash;
use betterlite_db::key_repository::VerificationTokenRepository;
use betterlite_db::key_repository::decode::{TokenDecodeError, decode_mapping};
use betterlite_db::key_repository::table::columns::TokenColumn;
use betterlite_db::key_repository::table::{RawTokenRow, TokenFilter, TokenValue};
use betterlite_db::postgres_store::session::PostgresStore;

use crate::key_fields::shared::hash_token_if_needed;
use crate::key_list_filters::postgres::PostgresTokenClient;
use crate::key_patch_fields::accessors::require_prisma_client;
use crate::key_patch_fields::error::KeyPatchError;
// ast-grep-ignore: no-long-names - the shared B726 attach keeps its name
use crate::object_permissions::attach::attach_object_permission_to_dict;

use super::error::KeyReassignmentError;

// _get_and_validate_existing_key: a supplied token wins over the alias, and the alias lookup asks for no relation
pub async fn get_and_validate_existing_key(
    token: Option<&str>,
    key_alias: Option<&str>,
    prisma_client: Option<PrismaClient<'_>>,
) -> Result<LiteLLMVerificationToken, KeyReassignmentError> {
    let client = require_prisma_client(prisma_client)?;
    // one snapshot keeps the row and the relation it carries on the same read side
    let availability = read_availability(client);
    let routed = PrismaClient {
        writer: client.writer,
        reader: client.reader,
        availability: &availability,
    };
    if let Some(supplied) = token {
        let hash = KeyHash::from(hash_token_if_needed(supplied));
        let rows = find_token_rows(routed, TokenLookup::Token(&hash)).await?;
        let Some(row) = rows.into_iter().next() else {
            return Err(KeyReassignmentError::from(KeyPatchError::KeyNotFound));
        };
        return decode_key(Some(routed), row).await;
    }
    let Some(alias) = key_alias else {
        return Err(KeyReassignmentError::MissingKeyIdentifier);
    };
    let rows = find_token_rows(routed, TokenLookup::Alias(alias)).await?;
    let [row] = rows.as_slice() else {
        return Err(match rows.len() {
            0 => KeyReassignmentError::AliasNotFound {
                alias: alias.to_owned(),
            },
            _ => KeyReassignmentError::AliasAmbiguous {
                alias: alias.to_owned(),
            },
        });
    };
    decode_key(None, row.clone()).await
}

fn read_availability(client: PrismaClient<'_>) -> Availability {
    let availability = client.availability;
    Availability {
        reader_unavailable: AtomicBool::new(availability.reader_unavailable.load(Ordering::SeqCst)),
        writer_unavailable: AtomicBool::new(availability.writer_unavailable.load(Ordering::SeqCst)),
    }
}

// the identifier the read starts from: a token is one unique row, an alias asks for two
#[derive(Clone, Copy)]
enum TokenLookup<'key> {
    Token(&'key KeyHash),
    Alias(&'key str),
}

async fn find_token_rows(
    client: PrismaClient<'_>,
    lookup: TokenLookup<'_>,
) -> Result<Vec<RawTokenRow>, KeyReassignmentError> {
    let availability = client.availability;
    let store = if availability.reader_unavailable.load(Ordering::SeqCst) {
        client.writer
    } else {
        client.reader
    };
    // the store is already the routed read side, so the repository needs no routing of its own
    let token_client = PostgresTokenClient { store };
    let repository = VerificationTokenRepository {
        database: Some(WriterPinnedClient::Writer(&token_client)),
    };
    let table = repository.table()?;
    match lookup {
        TokenLookup::Token(hash) => Ok(table.find_unique(hash).await?.into_iter().collect()),
        TokenLookup::Alias(alias) => {
            let filter = TokenFilter(vec![(
                TokenColumn::KeyAlias,
                TokenValue::Text(alias.to_owned()),
            )]);
            Ok(table.find_many(&filter, Some(2)).await?)
        }
    }
}

// include: {object_permission: true} belongs to the token lookup alone; an alias row keeps the relation absent
async fn decode_key(
    relation: Option<PrismaClient<'_>>,
    row: RawTokenRow,
) -> Result<LiteLLMVerificationToken, KeyReassignmentError> {
    let JsonData::Object(mut fields) = row.0 else {
        return Err(KeyReassignmentError::from(TokenDecodeError::NotAnObject));
    };
    if let Some(client) = relation {
        // ast-grep-ignore: no-long-names - the shared B726 attach keeps its name
        attach_object_permission_to_dict(&mut fields, Some(client)).await?;
    }
    decode_mapping(fields).map_err(KeyReassignmentError::from)
}
