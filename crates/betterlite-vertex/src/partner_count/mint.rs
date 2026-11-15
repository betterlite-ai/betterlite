use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

use crate::token_refresh::CredentialRefresh;
use crate::vertex_interactions::mint::{MintAccessToken, MintFuture};

// B1303's async refresh path, reached through the mint trait every Vertex call takes
impl MintAccessToken for CredentialRefresh {
    fn mint<'mint>(
        &'mint self,
        credentials: Option<&'mint ConfiguredCredentials>,
        project: Option<&'mint str>,
    ) -> MintFuture<'mint> {
        Box::pin(self.get_access_token_async(credentials, project))
    }
}
