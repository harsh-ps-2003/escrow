use std::env;
use std::fmt::Write;
use std::path::Path;

use anyhow::Context;
use devimint::federation::{Client, Federation};
use devimint::util::ProcessManager;
use devimint::{cmd, dev_fed, vars, DevFed};
use fedimint_core::task::TaskGroup;
use fedimint_core::util::write_overwrite_async;
use tokio::fs;
use tokio::sync::OnceCell;
use tracing::{debug, info};

static TRACING_SETUP: OnceCell<()> = OnceCell::const_new();

async fn setup() -> anyhow::Result<(ProcessManager, TaskGroup)> {
    let random_suffix: String = (0..8)
        .map(|_| (b'a' + (rand::random::<u8>() % 26)) as char)
        .collect();

    let test_dir = format!("{}{}", env::var("FM_TEST_DIR")?, random_suffix);
    let globals = vars::Global::new(
        Path::new(&test_dir),
        env::var("FM_FED_SIZE")?.parse::<usize>()?,
        0,
    )
    .await?;

    // tracing should be initialized only once for all tests
    TRACING_SETUP
        .get_or_init(|| async {
            let log_file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .append(true)
                .open(globals.FM_LOGS_DIR.join("devimint.log"))
                .await
                .expect("Failed to open log file")
                .into_std()
                .await;

            fedimint_logging::TracingSetup::default()
                .with_file(Some(log_file))
                .init()
                .expect("Failed to initialize tracing")
        })
        .await;

    let mut env_string = String::new();
    for (var, value) in globals.vars() {
        debug!(var, value, "Env variable set");
        writeln!(env_string, r#"export {var}="{value}""#)?; // hope that value doesn't contain a "
        unsafe {
            std::env::set_var(var, value);
        }
    }
    write_overwrite_async(globals.FM_TEST_DIR.join("env"), env_string).await?;
    info!("Test setup in {:?}", globals.FM_DATA_DIR);
    let process_mgr = ProcessManager::new(globals);
    let task_group = TaskGroup::new();
    task_group.install_kill_handler();
    Ok((process_mgr, task_group))
}

async fn setup_clients() -> anyhow::Result<(DevFed, Client, Client, Client, String, String)> {
    let (process_mgr, _) = setup().await?;

    let dev_fed = dev_fed(&process_mgr).await?;
    let fed = &dev_fed.fed;

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
    let seller_publickey = seller_pubkey["public_key"].as_str().unwrap().to_string();

    let arbiter_pubkey = cmd!(arbiter, "module", "escrow", "public-key")
        .out_json()
        .await?;
    let arbiter_publickey = arbiter_pubkey["public_key"].as_str().unwrap().to_string();

    Ok((
        dev_fed,
        buyer,
        seller,
        arbiter,
        seller_publickey,
        arbiter_publickey,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_test() -> anyhow::Result<()> {
    let (dev_fed, buyer, seller, arbiter, seller_pubkey, arbiter_pubkey) =
        setup_clients().await.context("failed to setup client")?;
    let fed = &dev_fed.fed;

    // Create escrow by buyer
    let cost = 50_000;
    let max_arbiter_fee_bps = 100; // 1%
    let create_result = cmd!(
        buyer,
        "module",
        "escrow",
        "create",
        &seller_pubkey,
        &arbiter_pubkey,
        &cost.to_string(),
        &max_arbiter_fee_bps.to_string()
    )
    .out_json()
    .await?;

    let escrow_id = create_result["escrow-id"].as_str().unwrap();
    let secret_code = create_result["secret-code"].as_str().unwrap();

    // Verify escrow info
    let escrow_info = cmd!(buyer, "module", "escrow", "info", escrow_id)
        .out_json()
        .await?;
    assert_eq!(escrow_info["state"].as_str().unwrap(), "Open");
    assert_eq!(escrow_info["amount"].as_u64().unwrap(), cost);

    // Seller claims escrow
    let claim_result = cmd!(seller, "module", "escrow", "claim", escrow_id, secret_code)
        .out_json()
        .await?;
    assert_eq!(claim_result["status"], "resolved");

    // Buyer attempts to claim escrow and fails
    let buyer_claim_result = cmd!(buyer, "module", "escrow", "claim", escrow_id, secret_code)
        .out_json()
        .await;
    assert!(buyer_claim_result.is_err());

    // Arbiter attempts to claim escrow and fails
    let arbiter_claim_result = cmd!(arbiter, "module", "escrow", "claim", escrow_id, secret_code)
        .out_json()
        .await;
    assert!(arbiter_claim_result.is_err());

    // Verify final balances
    assert_eq!(buyer.balance().await?, 99950000);
    assert_eq!(seller.balance().await?, 50_000);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unhappy_path_test() -> anyhow::Result<()> {
    let (dev_fed, buyer, seller, arbiter, seller_pubkey, arbiter_pubkey) =
        setup_clients().await.context("failed to setup client")?;
    let fed = &dev_fed.fed;

    // Create escrow
    let cost = 100_000;
    let max_arbiter_fee_bps = 200; // 2%
    let create_result = cmd!(
        buyer,
        "module",
        "escrow",
        "create",
        &seller_pubkey,
        &arbiter_pubkey,
        &cost.to_string(),
        &max_arbiter_fee_bps.to_string()
    )
    .out_json()
    .await?;

    let escrow_id = create_result["escrow-id"].as_str().unwrap().to_string();
    let secret_code = create_result["secret-code"].as_str().unwrap().to_string();

    // // Seller initiates dispute
    // let dispute_result = cmd!(seller, "module", "escrow", "dispute",
    // escrow_id.clone())     .out_json()
    //     .await?;
    // assert_eq!(dispute_result["status"], "disputed!");

    // let claim_result = cmd!(
    //     seller,
    //     "module",
    //     "escrow",
    //     "claim",
    //     escrow_id.clone(),
    //     secret_code.clone()
    // )
    // .out_json()
    // .await;
    // assert!(claim_result.is_err());

    // // cannot claim when the arbiter has not decided
    // let claim_after_dispute_result = cmd!(
    //     seller,
    //     "module",
    //     "escrow",
    //     "seller-claim",
    //     escrow_id.clone()
    // )
    // .out_json()
    // .await;
    // assert!(claim_after_dispute_result.is_err());

    // // arbiter has to make a valid decision
    // let invalid_decision_result = cmd!(
    //     arbiter,
    //     "module",
    //     "escrow",
    //     "arbiter-decision",
    //     escrow_id.clone(),
    //     "invalid_winner",
    //     "50"
    // )
    // .out_json()
    // .await;
    // assert!(invalid_decision_result.is_err());

    // // Arbiter makes decision in favor of seller
    // let arbiter_fee_bps = 50; // 0.5%
    // let decision_result = cmd!(
    //     arbiter,
    //     "module",
    //     "escrow",
    //     "arbiter-decision",
    //     escrow_id.clone(),
    //     "seller",
    //     &arbiter_fee_bps.to_string()
    // )
    // .out_json()
    // .await?;
    // println!("Decision result: {:?}", decision_result);
    // assert_eq!(decision_result["status"], "arbiter decision made!");

    // // Buyer cannot claim the escrow against arbiter decision
    // let claim_result = cmd!(buyer, "module", "escrow", "seller-claim",
    // escrow_id.clone())     .out_json()
    //     .await;
    // assert!(claim_result.is_err());

    // // Seller claims escrow
    // let claim_result = cmd!(
    //     seller,
    //     "module",
    //     "escrow",
    //     "seller-claim",
    //     escrow_id.clone()
    // )
    // .out_json()
    // .await?;
    // assert_eq!(claim_result["status"], "resolved!");

    // // Verify final balances
    // let arbiter_fee = (cost as f64 * (arbiter_fee_bps as f64 / 10000.0)) as u64;
    // assert_eq!(buyer.balance().await?, 99850000);
    // assert_eq!(seller.balance().await?, 49_750_000); // 50_000_000 - arbiter_fee
    // assert_eq!(arbiter.balance().await?, 250_000); // arbiter_fee

    Ok(())
}
