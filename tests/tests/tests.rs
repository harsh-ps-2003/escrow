use std::env;
use std::fmt::Write;
use std::path::Path;

use devimint::util::ProcessManager;
use devimint::{cmd, dev_fed, vars, DevFed};
use fedimint_core::task::TaskGroup;
use fedimint_core::util::write_overwrite_async;
use fedimint_testing::fixtures::Fixtures;
use tokio::fs;
use tracing::{debug, info};

async fn setup() -> anyhow::Result<(ProcessManager, TaskGroup)> {
    let globals = vars::Global::new(
        Path::new(&env::var("FM_TEST_DIR")?),
        env::var("FM_FED_SIZE")?.parse::<usize>()?,
        0,
    )
    .await?;
    let log_file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .append(true)
        .open(globals.FM_LOGS_DIR.join("devimint.log"))
        .await?
        .into_std()
        .await;

    fedimint_logging::TracingSetup::default()
        .with_file(Some(log_file))
        .init()?;

    let mut env_string = String::new();
    for (var, value) in globals.vars() {
        debug!(var, value, "Env variable set");
        writeln!(env_string, r#"export {var}="{value}""#)?; // hope that value doesn't contain a "
        std::env::set_var(var, value);
    }
    write_overwrite_async(globals.FM_TEST_DIR.join("env"), env_string).await?;
    info!("Test setup in {:?}", globals.FM_DATA_DIR);
    let process_mgr = ProcessManager::new(globals);
    let task_group = TaskGroup::new();
    task_group.install_kill_handler();
    Ok((process_mgr, task_group))
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_clients() -> anyhow::Result<()> {
    let (process_mgr, _) = setup().await?;

    let DevFed {
        bitcoind,
        cln,
        lnd,
        fed,
        gw_cln,
        gw_lnd,
        electrs,
        esplora,
    } = dev_fed(&process_mgr).await?;

    let buyer = fed.new_joined_client("fedimint-cli-buyer").await?;
    let seller = fed.new_joined_client("fedimint-cli-seller").await?;
    let arbiter = fed.new_joined_client("fedimint-cli-arbiter").await?;

    let initial_balance_sats = 100_000;
    fed.pegin_client(initial_balance_sats, &buyer).await?;

    // Verify balances
    assert_eq!(buyer.balance().await?, 100_000_000); // in msat
    assert_eq!(seller.balance().await?, 0);
    assert_eq!(arbiter.balance().await?, 0);

    // Get public keys
    let seller_pubkey = cmd!(seller, "module", "escrow", "public-key")
        .out_json()
        .await?;
    let seller_publickey = seller_pubkey["public_key"].as_str().unwrap();

    let arbiter_pubkey = cmd!("fedimint-cli-arbiter", "module", "escrow", "public-key")
        .out_json()
        .await?;
    let arbiter_publickey = arbiter_pubkey["public_key"].as_str().unwrap();

    Ok(())
}
