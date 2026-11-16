use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use betterlite_db::database_refresh::handle::PoolProvider;
use betterlite_db::database_refresh_loop::lifecycle::DatabaseRefreshLoop;
use betterlite_db::database_refresh_loop::minting::TokenMinter;
use tokio::task::JoinError;

pub type RefreshFuture<'service> =
    Pin<Box<dyn Future<Output = Result<(), JoinError>> + Send + 'service>>;

// B854 owns the loop; the composition starts it before serving and stops it on shutdown. The
// concrete minter's providers belong to B712, so the serving app injects the built duty.
pub trait CredentialRefresh: Send + Sync {
    fn start(&self);
    fn stop(&self) -> RefreshFuture<'_>;
}

impl<P, M, C> CredentialRefresh for DatabaseRefreshLoop<P, M, C>
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    fn start(&self) {
        <DatabaseRefreshLoop<P, M, C>>::start(self);
    }

    fn stop(&self) -> RefreshFuture<'_> {
        Box::pin(<DatabaseRefreshLoop<P, M, C>>::stop(self))
    }
}
