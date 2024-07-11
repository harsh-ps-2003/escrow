use fedimint_core::api::{FederationApiExt, IModuleFederationApi};
use fedimint_core::module::ApiRequestErased;
use fedimint_core::task::{MaybeSend, MaybeSync};
use fedimint_core::{apply, async_trait_maybe_send};
use fedimint_escrow_common::endpoints::{EscrowInfo, GET_MODULE_INFO};

#[apply(async_trait_maybe_send!)]
pub trait EscrowFederationApi: IModuleFederationApi {
    async fn get_escrow_info(&self, escrow_id: String) -> anyhow::Result<EscrowInfo>;
}

#[apply(async_trait_maybe_send!)]
impl<T: ?Sized> EscrowFederationApi for T
where
    T: IModuleFederationApi + MaybeSend + MaybeSync + 'static,
{
    async fn get_escrow_info(&self, escrow_id: String) -> anyhow::Result<EscrowInfo> {
        self.request_current_consensus(
            GET_MODULE_INFO.to_string(),
            ApiRequestErased::new(escrow_id),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Federation API error: {}", e))
    }
}
