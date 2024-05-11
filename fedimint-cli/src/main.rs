use fedimint_cli::FedimintCli;
use fedimint_core::fedimint_build_code_version_env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    FedimintCli::new(fedimint_build_code_version_env!())?
        .with_default_modules()
        .with_module(fedimint_escrow_client::EscrowClientInit)
        .run()
        .await;
    Ok(())
}
