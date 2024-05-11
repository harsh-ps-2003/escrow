use fedimint_core::core::ModuleKind;
use fedimint_core::fedimint_build_code_version_env;
use fedimint_escrow_common::config::EscrowGenParams;
use fedimint_escrow_server::EscrowInit;
use fedimintd::Fedimintd;

const KIND: ModuleKind = ModuleKind::from_static_str("escrow");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Fedimintd::new(fedimint_build_code_version_env!())?
        .with_default_modules()
        .with_module_kind(EscrowInit)
        .with_module_instance(KIND, EscrowGenParams::default())
        .run()
        .await
}
