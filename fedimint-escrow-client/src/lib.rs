pub mod api;
pub mod cli;
pub mod states;
#[cfg(feature = "cli")]
use std::sync::Arc;

use anyhow::Context as _;
use async_stream::stream;
use async_trait::async_trait;
use fedimint_client::module::init::{ClientModuleInit, ClientModuleInitArgs};
use fedimint_client::module::recovery::NoModuleBackup;
use fedimint_client::module::{ClientContext, ClientModule};
use fedimint_client::oplog::UpdateStreamOrOutcome;
use fedimint_client::transaction::{ClientInput, ClientOutput, TransactionBuilder};
use fedimint_core::api::DynModuleApi;
use fedimint_core::core::{KeyPair, OperationId};
use fedimint_core::db::{Database, DatabaseTransaction, DatabaseVersion};
use fedimint_core::module::{
    ApiVersion, ModuleCommon, ModuleInit, MultiApiVersion, TransactionItemAmount,
};
use fedimint_core::{apply, async_trait_maybe_send, Amount, OutPoint, TransactionId};
use fedimint_escrow_common::config::EscrowClientConfig;
use fedimint_escrow_common::endpoints::EscrowInfo;
use fedimint_escrow_common::{
    ArbiterDecision, EscrowCommonInit, EscrowError, EscrowInput, EscrowInputArbiterDecision,
    EscrowInputClaimingAfterDispute, EscrowInputClamingWithoutDispute, EscrowInputDisputing,
    EscrowModuleTypes, EscrowOutput, EscrowStates, KIND,
};
use futures::StreamExt;
use rand::{thread_rng, Rng};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use secp256k1::{Message, PublicKey, Secp256k1};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::api::EscrowFederationApi;
use crate::states::{EscrowClientContext, EscrowStateMachine};

/// The escrow client module
#[derive(Debug)]
pub struct EscrowClientModule {
    cfg: EscrowClientConfig,
    key: KeyPair,
    client_ctx: ClientContext<Self>,
    module_api: DynModuleApi,
    db: Database,
}

/// The high level state for tracking operations of transactions
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum EscrowOperationState {
    /// The transaction is being processed by the federation
    Created,
    /// The transaction is accepted by the federation
    Accepted,
    /// The transaction is rejected by the federation
    Rejected,
}

#[apply(async_trait_maybe_send!)]
impl ClientModule for EscrowClientModule {
    type Init = EscrowClientInit;
    type Common = EscrowModuleTypes;
    type Backup = NoModuleBackup;
    type ModuleStateMachineContext = EscrowClientContext;
    type States = EscrowStateMachine;

    fn context(&self) -> Self::ModuleStateMachineContext {
        EscrowClientContext {
            escrow_decoder: EscrowModuleTypes::decoder(),
        }
    }

    /// conveys the monetary value of escrow input
    fn input_amount(
        &self,
        input: &<Self::Common as ModuleCommon>::Input,
    ) -> Option<TransactionItemAmount> {
        match input {
            EscrowInput::ClamingWithoutDispute(input) => Some(TransactionItemAmount {
                amount: input.amount,
                fee: Amount::ZERO,
            }),
            EscrowInput::ClaimingAfterDispute(input) => Some(TransactionItemAmount {
                amount: input.amount,
                fee: Amount::ZERO,
            }),
            EscrowInput::ArbiterDecision(input) => Some(TransactionItemAmount {
                amount: input.amount,
                fee: Amount::ZERO,
            }),
            EscrowInput::Disputing(_) => None, // Disputing doesn't involve an amount
        }
    }

    /// conveys to the transaction the monetary value of escrow output so as to
    /// burn the equivalent ecash
    fn output_amount(
        &self,
        output: &<Self::Common as ModuleCommon>::Output,
    ) -> Option<TransactionItemAmount> {
        Some(TransactionItemAmount {
            amount: output.amount,
            fee: self.cfg.deposit_fee, /* deposit fee is required to use the escrow service to
                                        * avoid scams */
        })
    }

    #[cfg(feature = "cli")]
    async fn handle_cli_command(
        &self,
        args: &[std::ffi::OsString],
    ) -> anyhow::Result<serde_json::Value> {
        cli::handle_cli_command(self, args).await
    }
}

impl EscrowClientModule {
    /// Handles the buyer transaction for the escrow creation
    pub async fn create_escrow(
        &self,
        amount: Amount,
        seller_pubkey: PublicKey,
        arbiter_pubkey: PublicKey,
        escrow_id: String,
        secret_code_hash: String,
        max_arbiter_fee_bps: u16,
    ) -> anyhow::Result<(OperationId, OutPoint)> {
        let operation_id = OperationId(thread_rng().gen());

        // Validate max_arbiter_fee_bps (should be in range 10 to 1000)
        if let Err(e) = self.cfg.limit_max_arbiter_fee_bps() {
            return Err(anyhow::anyhow!("Invalid max_arbiter_fee_bps: {}", e));
        }

        // converting bps to percentage
        let fee_percentage = Decimal::from(max_arbiter_fee_bps) / Decimal::from(100);
        // getting the maximum arbiter fee that can be charged
        let max_arbiter_fee: Amount = Amount::from_sats(
            (Decimal::from(amount.msats) * fee_percentage / Decimal::from(1000))
                .to_u64()
                .unwrap(),
        );

        // creating output for buyers transaction by underfunding
        let output = EscrowOutput {
            amount,
            buyer_pubkey: self.key.public_key(),
            seller_pubkey,
            arbiter_pubkey,
            escrow_id,
            secret_code_hash,
            max_arbiter_fee,
        };

        let client_output = ClientOutput {
            output,
            state_machines: Arc::new(|_: TransactionId, _: u64| vec![EscrowStateMachine {}]),
        };

        // Build and send tx to the fed by underfunding the transaction
        // The transaction builder will select the necessary e-cash notes with mint
        // output to cover the output amount and create the corresponding inputs itself
        let tx = TransactionBuilder::new()
            .with_output(self.client_ctx.make_client_output(client_output));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        // Subscribe to transaction updates
        let mut updates = self
            .subscribe_transactions_output(operation_id, txid, change.clone())
            .await
            .unwrap()
            .into_stream();

        // Process the update stream
        while let Some(update) = updates.next().await {
            match update {
                EscrowOperationState::Created | EscrowOperationState::Accepted => {}
                EscrowOperationState::Rejected => {
                    return Err(anyhow::anyhow!(EscrowError::TransactionRejected));
                }
            }
        }

        Ok((operation_id, change[0]))
    }

    /// Handles the seller transaction to claim the funds that are locked in the
    /// escrow upon providing the secret code
    pub async fn claim_escrow(
        &self,
        escrow_id: String,
        amount: Amount,
        secret_code: String,
    ) -> anyhow::Result<()> {
        // make an api call to server db and get the secret code hash and state of
        // escrow, and then verify it
        let escrow_value: EscrowInfo = self.module_api.get_escrow_info(escrow_id.clone()).await?;
        // the escrow should not be in dispute when seller wants to claim
        if escrow_value.state == EscrowStates::DisputedByBuyer
            || escrow_value.state == EscrowStates::DisputedBySeller
        {
            return Err(anyhow::anyhow!(EscrowError::EscrowDisputed));
        }
        if escrow_value.state != EscrowStates::WaitingforSellerToClaim
            && escrow_value.state != EscrowStates::Open
        {
            return Err(anyhow::anyhow!(EscrowError::ArbiterNotDecided));
        }

        let secp = Secp256k1::new();
        // Hash the secret code string
        let mut hasher = Sha256::new();
        hasher.update(secret_code.as_bytes());
        let hashed_message: [u8; 32] = hasher.finalize().into();
        // Create the message from the hash
        let message = Message::from_slice(&hashed_message).expect("32 bytes");
        // Sign the message using Schnorr signature
        let signature = secp.sign_schnorr(&message, &self.key);

        let operation_id = OperationId(thread_rng().gen());
        // Transfer ecash to seller by overfunding the transaction
        // Create input using the buyer account
        let input = EscrowInput::ClamingWithoutDispute(EscrowInputClamingWithoutDispute {
            amount,
            escrow_id,
            secret_code: secret_code,
            hashed_message: hashed_message,
            signature: signature,
        });

        let client_input = ClientInput {
            input,
            keys: vec![],
            state_machines: Arc::new(|_: TransactionId, _: u64| vec![EscrowStateMachine {}]),
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx =
            TransactionBuilder::new().with_input(self.client_ctx.make_client_input(client_input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, _change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        // Subscribe to transaction updates
        let mut updates = self
            .subscribe_transactions_input(operation_id, txid)
            .await
            .unwrap()
            .into_stream();

        // Process the update stream
        while let Some(update) = updates.next().await {
            match update {
                EscrowOperationState::Created | EscrowOperationState::Accepted => {}
                EscrowOperationState::Rejected => {
                    return Err(anyhow::anyhow!(EscrowError::TransactionRejected));
                }
            }
        }

        Ok(())
    }

    /// Handles the claiming of ecash by the buyer after the arbiter has decided
    /// that buyer won the dispute
    pub async fn buyer_claim(&self, escrow_id: String, amount: Amount) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());

        let escrow_value: EscrowInfo = self.module_api.get_escrow_info(escrow_id.clone()).await?;
        // the arbiter has not decided yet if the escrow is disputed!
        if escrow_value.state == EscrowStates::DisputedByBuyer
            || escrow_value.state == EscrowStates::DisputedBySeller
        {
            return Err(anyhow::anyhow!(EscrowError::ArbiterNotDecided));
        }
        // the state should be waiting for buyer to claim the ecash as arbiter has
        // decided
        if escrow_value.state != EscrowStates::WaitingforBuyerToClaim {
            return Err(anyhow::anyhow!(EscrowError::ArbiterNotDecided));
        }

        let secp = Secp256k1::new();
        // Hash the decision string
        let mut hasher = Sha256::new();
        hasher.update("buyer_claim".as_bytes());
        let hashed_message: [u8; 32] = hasher.finalize().into();
        // Create the message from the hash
        let message = Message::from_slice(&hashed_message).expect("32 bytes");
        // Sign the message using Schnorr signature
        let signature = secp.sign_schnorr(&message, &self.key);

        // Transfer ecash back to buyer after deduction of arbiter fee by underfunding
        // the transaction
        let input = EscrowInput::ClaimingAfterDispute(EscrowInputClaimingAfterDispute {
            amount,
            escrow_id,
            hashed_message: hashed_message,
            signature: signature,
        });

        let client_input = ClientInput {
            input,
            keys: vec![],
            state_machines: Arc::new(|_: TransactionId, _: u64| vec![EscrowStateMachine {}]),
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx =
            TransactionBuilder::new().with_input(self.client_ctx.make_client_input(client_input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, _change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        // Subscribe to transaction updates
        let mut updates = self
            .subscribe_transactions_input(operation_id, txid)
            .await
            .unwrap()
            .into_stream();

        // Process the update stream
        while let Some(update) = updates.next().await {
            match update {
                EscrowOperationState::Created | EscrowOperationState::Accepted => {}
                EscrowOperationState::Rejected => {
                    return Err(anyhow::anyhow!(EscrowError::TransactionRejected));
                }
            }
        }

        Ok(())
    }

    /// Handles the claiming of transaction by the seller after the arbiter has
    /// decided that seller won the dispute
    pub async fn seller_claim(&self, escrow_id: String, amount: Amount) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());

        let escrow_value: EscrowInfo = self.module_api.get_escrow_info(escrow_id.clone()).await?;
        // the arbiter has not decided yet if the escrow is disputed!
        if escrow_value.state == EscrowStates::DisputedByBuyer
            || escrow_value.state == EscrowStates::DisputedBySeller
        {
            return Err(anyhow::anyhow!(EscrowError::ArbiterNotDecided));
        }
        // the state should be waiting for seller to claim the ecash as arbiter has
        // decided
        if escrow_value.state != EscrowStates::WaitingforSellerToClaim {
            return Err(anyhow::anyhow!(EscrowError::ArbiterNotDecided));
        }

        let secp = Secp256k1::new();
        // Hash the decision string
        let mut hasher = Sha256::new();
        hasher.update("seller_claim".as_bytes());
        let hashed_message: [u8; 32] = hasher.finalize().into();
        // Create the message from the hash
        let message = Message::from_slice(&hashed_message).expect("32 bytes");
        // Sign the message using Schnorr signature
        let signature = secp.sign_schnorr(&message, &self.key);

        // Transfer ecash back to buyer by underfunding the transaction
        let input = EscrowInput::ClaimingAfterDispute(EscrowInputClaimingAfterDispute {
            amount,
            escrow_id,
            hashed_message: hashed_message,
            signature: signature,
        });

        let client_input = ClientInput {
            input,
            keys: vec![],
            state_machines: Arc::new(|_: TransactionId, _: u64| vec![EscrowStateMachine {}]),
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx =
            TransactionBuilder::new().with_input(self.client_ctx.make_client_input(client_input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, _change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        // Subscribe to transaction updates
        let mut updates = self
            .subscribe_transactions_input(operation_id, txid)
            .await
            .unwrap()
            .into_stream();

        // Process the update stream
        while let Some(update) = updates.next().await {
            match update {
                EscrowOperationState::Created | EscrowOperationState::Accepted => {}
                EscrowOperationState::Rejected => {
                    return Err(anyhow::anyhow!(EscrowError::TransactionRejected));
                }
            }
        }

        Ok(())
    }

    /// Handles the initiation of dispute
    pub async fn initiate_dispute(&self, escrow_id: String) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());

        let secp = Secp256k1::new();
        // Hash the decision string
        let mut hasher = Sha256::new();
        hasher.update("dispute".as_bytes());
        let hashed_message: [u8; 32] = hasher.finalize().into();
        // Create the message from the hash
        let message = Message::from_slice(&hashed_message).expect("32 bytes");
        // Sign the message using Schnorr signature using disputers keypair
        let signature = secp.sign_schnorr(&message, &self.key);

        let input = EscrowInput::Disputing(EscrowInputDisputing {
            escrow_id: escrow_id,
            disputer: self.key.public_key(),
            hashed_message: hashed_message,
            signature: signature,
        });

        let client_input = ClientInput {
            input,
            keys: vec![],
            state_machines: Arc::new(|_: TransactionId, _: u64| vec![EscrowStateMachine {}]),
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx =
            TransactionBuilder::new().with_input(self.client_ctx.make_client_input(client_input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, _change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        // Subscribe to transaction updates
        let mut updates = self
            .subscribe_transactions_input(operation_id, txid)
            .await
            .unwrap()
            .into_stream();

        // Process the update stream
        while let Some(update) = updates.next().await {
            match update {
                EscrowOperationState::Created | EscrowOperationState::Accepted => {}
                EscrowOperationState::Rejected => {
                    return Err(anyhow::anyhow!(EscrowError::TransactionRejected));
                }
            }
        }

        Ok(())
    }

    /// Handles the arbiter decision making on who won the dispute
    pub async fn arbiter_decision(
        &self,
        escrow_id: String,
        decision: String,
        arbiter_fee_bps: u16,
    ) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());

        let arbiter_decision = match decision.to_lowercase().as_str() {
            "buyer" => ArbiterDecision::BuyerWins,
            "seller" => ArbiterDecision::SellerWins,
            _ => return Err(anyhow::anyhow!(EscrowError::InvalidArbiterDecision)),
        };

        let escrow_value: EscrowInfo = self.module_api.get_escrow_info(escrow_id.clone()).await?;

        // getting percentage from basis points
        let fee_percentage = Decimal::from(arbiter_fee_bps) / Decimal::from(100);
        // calculating arbiter fee
        let arbiter_fee: Amount = Amount::from_sats(
            (Decimal::from(escrow_value.amount.msats) * fee_percentage / Decimal::from(1000))
                .to_u64()
                .unwrap(),
        );

        let secp = Secp256k1::new();
        // Hash the decision string
        let mut hasher = Sha256::new();
        hasher.update(decision.as_bytes());
        let hashed_message: [u8; 32] = hasher.finalize().into();
        // Create the message from the hash
        let message = Message::from_slice(&hashed_message).expect("32 bytes");
        // Sign the message using Schnorr signature
        let signature = secp.sign_schnorr(&message, &self.key);

        // Transfer ecash back to buyer by underfunding the transaction
        let input = EscrowInput::ArbiterDecision(EscrowInputArbiterDecision {
            amount: arbiter_fee,
            escrow_id: escrow_id,
            arbiter_decision,
            hashed_message: hashed_message,
            signature: signature,
        });

        let client_input = ClientInput {
            input,
            keys: vec![],
            state_machines: Arc::new(|_: TransactionId, _: u64| vec![EscrowStateMachine {}]),
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx =
            TransactionBuilder::new().with_input(self.client_ctx.make_client_input(client_input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, _change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        // Subscribe to transaction updates
        let mut updates = self
            .subscribe_transactions_input(operation_id, txid)
            .await
            .unwrap()
            .into_stream();

        // Process the update stream
        while let Some(update) = updates.next().await {
            match update {
                EscrowOperationState::Created | EscrowOperationState::Accepted => {}
                EscrowOperationState::Rejected => {
                    return Err(anyhow::anyhow!(EscrowError::TransactionRejected));
                }
            }
        }

        Ok(())
    }

    /// Subscribes to the transaction updates and yields the state of operation,
    /// when the transaction has input attached not output!
    pub async fn subscribe_transactions_input(
        &self,
        operation_id: OperationId,
        txid: TransactionId,
    ) -> anyhow::Result<UpdateStreamOrOutcome<EscrowOperationState>> {
        let tx_subscription = self.client_ctx.transaction_updates(operation_id).await;

        Ok(UpdateStreamOrOutcome::UpdateStream(Box::pin(stream! {
            yield EscrowOperationState::Created;

            match tx_subscription.await_tx_accepted(txid).await {
                Ok(()) => {
                    yield EscrowOperationState::Accepted;
                },
                Err(_) => {
                    yield EscrowOperationState::Rejected;
                }
            }
        })))
    }

    /// Subscribes to the transaction updates and yields the state of operation
    /// when the transaction has output attached not input!
    pub async fn subscribe_transactions_output(
        &self,
        operation_id: OperationId,
        txid: TransactionId,
        change: Vec<OutPoint>,
    ) -> anyhow::Result<UpdateStreamOrOutcome<EscrowOperationState>> {
        let tx_subscription = self.client_ctx.transaction_updates(operation_id).await;
        let client_ctx = self.client_ctx.clone();

        Ok(UpdateStreamOrOutcome::UpdateStream(Box::pin(stream! {
            yield EscrowOperationState::Created;

            match tx_subscription.await_tx_accepted(txid).await {
                Ok(()) => {
                    // when the transaction has ecash output, we need to make sure its claimed
                    match client_ctx
                        .await_primary_module_outputs(operation_id, change)
                        .await
                        .context("Ensuring that the transaction is successful!") {
                        Ok(_) => yield EscrowOperationState::Accepted,
                        Err(_) => yield EscrowOperationState::Rejected,
                    }
                },
                Err(_) => {
                    yield EscrowOperationState::Rejected;
                }
            }
        })))
    }
}

/// The escrow client module initializer
#[derive(Debug, Clone)]
pub struct EscrowClientInit;

#[async_trait]
impl ModuleInit for EscrowClientInit {
    type Common = EscrowCommonInit;
    const DATABASE_VERSION: DatabaseVersion = DatabaseVersion(0);

    async fn dump_database(
        &self,
        _dbtx: &mut DatabaseTransaction<'_>,
        _prefix_names: Vec<String>,
    ) -> Box<dyn Iterator<Item = (String, Box<dyn erased_serde::Serialize + Send>)> + '_> {
        Box::new(std::iter::empty())
    }
}

/// Generates the client module
#[apply(async_trait_maybe_send!)]
impl ClientModuleInit for EscrowClientInit {
    type Module = EscrowClientModule;

    fn supported_api_versions(&self) -> MultiApiVersion {
        MultiApiVersion::try_from_iter([ApiVersion { major: 0, minor: 0 }])
            .expect("no version conflicts")
    }

    async fn init(&self, args: &ClientModuleInitArgs<Self>) -> anyhow::Result<Self::Module> {
        let cfg = args.cfg().clone();
        Ok(EscrowClientModule {
            cfg: cfg.clone(),
            module_api: args.module_api().clone(),
            key: args
                .module_root_secret()
                .clone()
                .to_secp_key(&Secp256k1::new()),
            client_ctx: args.context(),
            db: args.db().clone(),
        })
    }
}
