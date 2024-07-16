// use std::sync::Arc;

// use anyhow::bail;
// use fedimint_client::transaction::{ClientInput, ClientOutput,
// TransactionBuilder}; use fedimint_core::config::ClientModuleConfig;
// use fedimint_core::core::{IntoDynInstance, ModuleKind, OperationId};
// use fedimint_core::db::mem_impl::MemDatabase;
// use fedimint_core::module::ModuleConsensusVersion;
// use fedimint_core::{sats, Amount, OutPoint};
// use fedimint_escrow_client::{EscrowClientInit, EscrowClientModule,
// EscrowError}; use fedimint_escrow_common::config::{EscrowClientConfig,
// EscrowGenParams}; use fedimint_escrow_common::{EscrowInput, EscrowOutput,
// KIND}; use fedimint_escrow_common::endpoints::EscrowInfo;
// use fedimint_escrow_server::EscrowInit;
// use fedimint_testing::fixtures::Fixtures;
// use secp256k1::Secp256k1;

// async fn setup_test_env(
//     initial_amount: Amount,
//     max_arbiter_fee_bps: u16,
// ) -> anyhow::Result<(
//     Client,
//     Client,
//     Client,
//     EscrowClientModule,
//     EscrowClientModule,
//     EscrowClientModule,
//     String,
//     String,
// )> {
//     let fed = fixtures().new_fed().await;
//     let (buyer, seller) = fed.two_clients().await;
//     let arbiter = fed.new_client().await;

//     let buyer_escrow = buyer.get_first_module::<EscrowClientModule>();
//     let seller_escrow = seller.get_first_module::<EscrowClientModule>();
//     let arbiter_escrow = arbiter.get_first_module::<EscrowClientModule>();

//     buyer.fund(initial_amount).await?;

//     let escrow_id = "escrow_id".to_string();
//     let secret_code = "secret_code".to_string();
//     let secret_code_hash = hash256(secret_code.clone());

//     buyer_escrow
//         .create_escrow(
//             initial_amount,
//             seller.public_key(),
//             arbiter.public_key(),
//             escrow_id.clone(),
//             secret_code_hash,
//             max_arbiter_fee_bps,
//         )
//         .await?;

//     Ok((
//         buyer,
//         seller,
//         arbiter,
//         buyer_escrow,
//         seller_escrow,
//         arbiter_escrow,
//         escrow_id,
//         secret_code,
//     ))
// }

// // Implement a mock get_escrow_info for testing
// impl EscrowClientModule {
//     async fn mock_get_escrow_info(&self, escrow_id: String) ->
// anyhow::Result<EscrowInfo> {         // This is a simplified mock
// implementation. In a real scenario, you'd want to store and retrieve actual
// escrow data.         Ok(EscrowInfo {
//             buyer_pubkey: self.key.public_key(),
//             seller_pubkey: self.key.public_key(), // For simplicity, using
// the same key             arbiter_pubkey: self.key.public_key(), // For
// simplicity, using the same key             amount: Amount::sats(1000),
//             state: "open".to_string(),
//             max_arbiter_fee_bps: 20,
//         })
//     }
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_create_escrow() -> anyhow::Result<()> {
//     let (buyer, seller, arbiter, buyer_escrow, _, _, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let escrow_info =
// buyer_escrow.mock_get_escrow_info(escrow_id.clone()).await?;     assert_eq!
// (escrow_info.buyer_pubkey, buyer.public_key());     assert_eq!(escrow_info.
// seller_pubkey, seller.public_key());     assert_eq!(escrow_info.
// arbiter_pubkey, arbiter.public_key());     assert_eq!(escrow_info.amount,
// Amount::sats(1000));     assert_eq!(escrow_info.state, "open");
//     assert_eq!(escrow_info.max_arbiter_fee_bps, 20);

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_seller_claim_with_correct_secret() -> anyhow::Result<()> {
//     let (_, _, _, _, seller_escrow, _, escrow_id, secret_code) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let result = seller_escrow.claim_escrow(escrow_id, secret_code,
// Amount::sats(1000)).await;     assert!(result.is_ok());

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_seller_claim_with_incorrect_secret() -> anyhow::Result<()> {
//     let (_, _, _, _, seller_escrow, _, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let incorrect_secret = "incorrect_secret".to_string();
//     let result = seller_escrow.claim_escrow(escrow_id, incorrect_secret,
// Amount::sats(1000)).await;     assert!(matches!(result,
// Err(EscrowError::InvalidSecretCode)));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_buyer_initiate_dispute() -> anyhow::Result<()> {
//     let (_, _, _, buyer_escrow, _, _, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let result = buyer_escrow.initiate_dispute(escrow_id.clone()).await;
//     assert!(result.is_ok());

//     let escrow_info = buyer_escrow.mock_get_escrow_info(escrow_id).await?;
//     assert_eq!(escrow_info.state, "disputed");

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_arbiter_decision_buyer_wins() -> anyhow::Result<()> {
//     let (buyer, _, _, buyer_escrow, _, arbiter_escrow, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;
//     arbiter_escrow.arbiter_decision(escrow_id.clone(), "buyer").await?;

//     let escrow_info = buyer_escrow.mock_get_escrow_info(escrow_id).await?;
//     assert_eq!(escrow_info.state, "resolved_buyer_wins");

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_arbiter_decision_seller_wins() -> anyhow::Result<()> {
//     let (_, seller, _, buyer_escrow, seller_escrow, arbiter_escrow,
// escrow_id, _) = setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;
//     arbiter_escrow.arbiter_decision(escrow_id.clone(), "seller").await?;

//     let escrow_info = seller_escrow.mock_get_escrow_info(escrow_id).await?;
//     assert_eq!(escrow_info.state, "resolved_seller_wins");

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_buyer_claim_after_winning_dispute() -> anyhow::Result<()> {
//     let (buyer, _, _, buyer_escrow, _, arbiter_escrow, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;
//     arbiter_escrow.arbiter_decision(escrow_id.clone(), "buyer").await?;

//     let result = buyer_escrow.buyer_claim(escrow_id,
// Amount::sats(900)).await;     assert!(result.is_ok());

//     assert_eq!(buyer.get_balance().await, Amount::sats(900));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_seller_claim_after_winning_dispute() -> anyhow::Result<()> {
//     let (_, seller, _, buyer_escrow, seller_escrow, arbiter_escrow,
// escrow_id, _) = setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;
//     arbiter_escrow.arbiter_decision(escrow_id.clone(), "seller").await?;

//     let result = seller_escrow.claim_escrow(escrow_id,
// "secret_code".to_string(), Amount::sats(900)).await;     assert!(result.
// is_ok());

//     assert_eq!(seller.get_balance().await, Amount::sats(900));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_cannot_claim_disputed_escrow_before_resolution() ->
// anyhow::Result<()> {     let (_, _, _, buyer_escrow, seller_escrow, _,
// escrow_id, _) = setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;

//     let result = seller_escrow.claim_escrow(escrow_id,
// "secret_code".to_string(), Amount::sats(1000)).await;     assert!(matches!
// (result, Err(EscrowError::EscrowDisputed)));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_cannot_dispute_already_disputed_escrow() -> anyhow::Result<()>
// {     let (_, _, _, buyer_escrow, seller_escrow, _, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;

//     let result = seller_escrow.seller_dispute(escrow_id).await;
//     assert!(matches!(result, Err(EscrowError::EscrowAlreadyDisputed)));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_cannot_make_arbiter_decision_on_undisputed_escrow() ->
// anyhow::Result<()> {     let (_, _, _, _, _, arbiter_escrow, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let result = arbiter_escrow.arbiter_decision(escrow_id, "buyer").await;
//     assert!(matches!(result, Err(EscrowError::EscrowNotDisputed)));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_create_escrow_with_max_arbiter_fee() -> anyhow::Result<()> {
//     let (buyer, seller, arbiter, buyer_escrow, _, _, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let escrow_info =
// buyer_escrow.mock_get_escrow_info(escrow_id.clone()).await?;     assert_eq!
// (escrow_info.max_arbiter_fee_bps, 20);

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_arbiter_decision_with_fee() -> anyhow::Result<()> {
//     let (buyer, seller, _, buyer_escrow, seller_escrow, arbiter_escrow,
// escrow_id, _) = setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;
//     arbiter_escrow.arbiter_decision(escrow_id.clone(), "seller", 10).await?;

//     let escrow_info =
// seller_escrow.mock_get_escrow_info(escrow_id.clone()).await?;     assert_eq!
// (escrow_info.state, "resolved_seller_wins");

//     // Seller should receive 990 sats (1000 - 1% arbiter fee)
//     seller_escrow.claim_escrow(escrow_id, "secret_code".to_string(),
// Amount::sats(990)).await?;     assert_eq!(seller.get_balance().await,
// Amount::sats(990));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_seller_claim() -> anyhow::Result<()> {
//     let (_, seller, _, _, seller_escrow, _, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let result = seller_escrow.seller_claim(escrow_id.clone(),
// Amount::sats(1000)).await;     assert!(result.is_ok());

//     assert_eq!(seller.get_balance().await, Amount::sats(1000));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_public_key() -> anyhow::Result<()> {
//     let (_, _, _, buyer_escrow, _, _, _, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     let public_key = buyer_escrow.public_key();
//     assert!(public_key.is_some());

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_claim_already_claimed_escrow() -> anyhow::Result<()> {
//     let (_, _, _, _, seller_escrow, _, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     seller_escrow.claim_escrow(escrow_id.clone(), "secret_code".to_string(),
// Amount::sats(1000)).await?;

//     let result = seller_escrow.claim_escrow(escrow_id,
// "secret_code".to_string(), Amount::sats(1000)).await;     assert!(matches!
// (result, Err(EscrowError::EscrowAlreadyClaimed)));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_multiple_arbiter_decisions() -> anyhow::Result<()> {
//     let (_, _, _, buyer_escrow, _, arbiter_escrow, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;
//     arbiter_escrow.arbiter_decision(escrow_id.clone(), "buyer", 10).await?;

//     let result = arbiter_escrow.arbiter_decision(escrow_id, "seller",
// 10).await;     assert!(matches!(result,
// Err(EscrowError::EscrowAlreadyResolved)));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_create_escrow_with_insufficient_funds() -> anyhow::Result<()> {
//     let (_, _, _, buyer_escrow, _, _, escrow_id, _) =
// setup_test_env(Amount::sats(500), 20).await?;

//     let result = buyer_escrow.create_escrow(
//         Amount::sats(1000),
//         buyer_escrow.key.public_key(),
//         buyer_escrow.key.public_key(),
//         escrow_id,
//         hash256("secret".to_string()),
//         20,
//     ).await;

//     assert!(matches!(result, Err(EscrowError::InsufficientFunds)));

//     Ok(())
// }

// #[tokio::test(flavor = "multi_thread")]
// async fn test_arbiter_decision_with_excessive_fee() -> anyhow::Result<()> {
//     let (_, _, _, buyer_escrow, _, arbiter_escrow, escrow_id, _) =
// setup_test_env(Amount::sats(1100), 20).await?;

//     buyer_escrow.initiate_dispute(escrow_id.clone()).await?;
//     let result = arbiter_escrow.arbiter_decision(escrow_id,
// "seller".to_string(), 30).await;

//     assert!(matches!(result, Err(EscrowError::ExcessiveArbiterFee)));

//     Ok(())
// }
