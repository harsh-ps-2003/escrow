use std::sync::Arc;

use anyhow::bail;
use fedimint_client::transaction::{ClientInput, ClientOutput, TransactionBuilder};
use fedimint_core::config::ClientModuleConfig;
use fedimint_core::core::{IntoDynInstance, ModuleKind, OperationId};
use fedimint_core::db::mem_impl::MemDatabase;
use fedimint_core::module::ModuleConsensusVersion;
use fedimint_core::{sats, Amount, OutPoint};
use fedimint_escrow_client::states::EscrowStateMachine;
use fedimint_escrow_client::{EscrowClientInit, EscrowClientModule};
use fedimint_escrow_common::config::{EscrowClientConfig, EscrowGenParams};
use fedimint_escrow_common::{EscrowInput, EscrowOutput, KIND};
use fedimint_escrow_server::EscrowInit;
use fedimint_testing::fixtures::Fixtures;
use secp256k1::Secp256k1;

fn fixtures() -> Fixtures {
    Fixtures::new_primary(EscrowClientInit, EscrowInit, EscrowGenParams::default())
}

async fn setup_test_env() -> anyhow::Result<(
    Client,
    Client,
    Client,
    EscrowClientModule,
    EscrowClientModule,
)> {
    let fed = fixtures().new_fed().await;
    let (buyer, seller) = fed.two_clients().await;
    let arbiter = fed.new_client().await;

    // when buyer needs to interact with escrow
    let buyer_escrow = buyer.get_first_module::<EscrowClientModule>();
    // when seller needs to interact with escrow
    let seller_escrow = seller.get_first_module::<EscrowClientModule>();
    // when arbiter needs to interact with escrow
    let arbiter_escrow = arbiter.get_first_module::<EscrowClientModule>();

    // Fund the buyer with 1100 sats
    buyer.fund(sats(1100)).await?;

    // buyer creates escrow
    let (op_id, outpoint, escrow_id) = buyer_escrow
        .create_escrow(
            Amount::sats(1000),
            seller.public_key(),
            arbiter.public_key(),
            3600, // 1 hour retreat duration
        )
        .await?;

    Ok((
        buyer,
        seller,
        arbiter,
        buyer_escrow,
        seller_escrow,
        arbiter_escrow,
        escrow_id,
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn get_module_info_returns_expected() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Prepare arguments for the EscrowInfo command
    let args = vec![
        ffi::OsString::from("EscrowInfo"),
        ffi::OsString::from(escrow_id.clone()),
    ];

    // Call the handle_cli_command function from cli.rs
    let response = handle_cli_command(&buyer_escrow, &args).await?;

    // expected JSON response
    let expected_json = json!({
        "buyer": buyer.public_key().to_string(),
        "seller": seller.public_key().to_string(),
        "arbiter": arbiter.public_key().to_string(),
        "escrow_id": escrow_id,
        "status": "open", // Assuming the status is 'open' initially
        "amount": amount,
        "retreat_duration": 3600
    });

    // Assert that the response matches the expected JSON
    assert_eq!(response, expected_json);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn can_create_and_claim_escrow_in_happy_state() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Seller claims escrow with secret code
    let secret_code: String = "secret123".to_string();
    seller_escrow
        .seller_txn(escrow_id, secret_code, amount)
        .await?;

    // Check balances
    assert_eq!(buyer.get_balance().await, Amount::ZERO);
    assert_eq!(seller.get_balance().await, sats(1000));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn can_dispute_and_resolve_escrow_in_favor_of_buyer() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Buyer disputes escrow
    buyer_escrow.initiate_dispute(escrow_id, sats(100)).await?;

    // Arbiter resolves dispute in favor of buyer
    arbiter_escrow.arbiter_txn(escrow_id, "buyer").await?;

    // Buyer retreats funds but paid arbiter from his pocket
    buyer_escrow.retreat_txn(escrow_id, sats(900)).await?;

    // Check balances
    assert_eq!(buyer.get_balance().await, sats(900)); // minus arbiter fee
    assert_eq!(seller.get_balance().await, Amount::ZERO);
    assert_eq!(arbiter.get_balance().await, sats(100));

    Ok(())
}

// buyer disputed the escrow but seller won!
#[tokio::test(flavor = "multi_thread")]
async fn can_dispute_and_resolve_escrow_in_favor_of_seller() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Buyer disputes escrow
    buyer_escrow.initiate_dispute(escrow_id, sats(100)).await?;

    // Arbiter resolves dispute in favor of buyer
    arbiter_escrow.arbiter_txn(escrow_id, "seller").await?;

    // Seller claims disputed funds
    seller_escrow
        .seller_txn(escrow_id, secret_code, sats(900))
        .await?;

    // Check balances
    assert_eq!(buyer.get_balance().await, Amount::ZERO); // minus arbiter fee
    assert_eq!(seller.get_balance().await, sats(1000));
    assert_eq!(arbiter.get_balance().await, sats(100));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn retreat_fails_before_time_passed() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Buyer tries to retreat before retreat duration has passed
    let res = buyer_escrow.buyer_retreat(escrow_id).await;

    // Check it returns RetreatTimeNotPassed error
    assert!(matches!(res, Err(EscrowError::RetreatTimeNotPassed)));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_secret_code_fails_claim() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Seller tries to claim with invalid secret code
    let invalid_code: String = "wrong_secret".to_string();
    let res = seller_escrow
        .seller_txn(escrow_id, invalid_code, amount)
        .await;

    // Check that it returns InvalidSecretCode error
    assert!(matches!(res, Err(EscrowError::InvalidSecretCode)));
}

#[tokio::test(flavor = "multi_thread")]
async fn claim_fails_when_disputed() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Buyer disputes escrow
    buyer_escrow.initiate_dispute(escrow_id, sats(100)).await?;

    // Seller tries to claim
    let res = seller_escrow
        .seller_txn(escrow_id, secret_code, amount)
        .await;

    // Check it returns EscrowDisputed error
    assert!(matches!(res, Err(EscrowError::EscrowDisputed)));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn non_arbiter_cannot_resolve() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Dispute the escrow
    seller_escrow.seller_dispute(escrow_id).await?;

    // Non-arbiter client tries to resolve
    let non_arbiter = fed.new_client().await;
    let res = non_arbiter
        .get_first_module::<EscrowClientModule>()
        .arbiter_resolve(escrow_id, seller.public_key())
        .await;

    // Check it returns ArbiterNotMatched error
    assert!(matches!(res, Err(EscrowError::ArbiterNotMatched)));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cannot_retreat_before_arbiter_resolves() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Dispute the escrow
    seller_escrow.seller_dispute(escrow_id).await?;

    // Buyer tries to retreat before arbiter resolves
    let res = buyer_escrow.buyer_retreat(escrow_id).await;

    // Check it returns ArbiterNotDecided error
    assert!(matches!(res, Err(EscrowError::ArbiterNotDecided)));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn arbiter_decision_fails_when_not_disputed() -> anyhow::Result<()> {
    let (buyer, seller, arbiter, buyer_escrow, seller_escrow, arbiter_escrow, escrow_id) =
        setup_test_env().await?;

    // Attempt to make an arbiter decision when no dispute has been raised
    let res = arbiter_escrow.arbiter_txn(escrow_id, "buyer").await;

    // Check it returns EscrowNotDisputed error
    assert!(matches!(res, Err(EscrowError::EscrowNotDisputed)));

    Ok(())
}
