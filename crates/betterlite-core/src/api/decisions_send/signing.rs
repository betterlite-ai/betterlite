use std::future::Future;
use std::pin::Pin;

use betterlite_openai::openai_base_config::Headers;
use tokio::runtime::{Handle, RuntimeFlavor};
use tokio::task::block_in_place;

pub(super) type Signed = (Headers, Option<Vec<u8>>);
pub(super) type SignFuture<'seam> = Pin<Box<dyn Future<Output = Signed> + Send + 'seam>>;
pub(super) type SignTask<'seam> = &'seam (dyn Fn() -> Signed + Sync);

// B595's sign_request_off_loop_if_aws: an AWS-signed config signs off the request task, every other one inline
pub(super) trait SigningSeam: Send + Sync {
    fn sign<'seam>(&'seam self, off_loop: bool, sign: SignTask<'seam>) -> SignFuture<'seam>;
}

// The production seam: block_in_place tells the runtime this worker is about to block, so it moves the worker's other tasks to another thread
pub(super) struct BlockingSigning;

impl SigningSeam for BlockingSigning {
    fn sign<'seam>(&'seam self, off_loop: bool, sign: SignTask<'seam>) -> SignFuture<'seam> {
        Box::pin(async move {
            let multithreaded = Handle::try_current()
                .is_ok_and(|handle| handle.runtime_flavor() == RuntimeFlavor::MultiThread);
            // block_in_place panics on a current-thread runtime, where signing stays inline like the sync path
            if !off_loop || !multithreaded {
                return sign();
            }
            block_in_place(sign)
        })
    }
}
