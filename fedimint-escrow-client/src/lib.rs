use std::sync::Arc;

use anyhow::{anyhow, Context as _};
use fedimint_client::module::init::{ClientModuleInit, ClientModuleInitArgs};
use fedimint_client::module::recovery::NoModuleBackup;
use fedimint_client::module::{ClientContext, ClientModule};
use fedimint_client::sm::{Context, ModuleNotifier};
use fedimint_client::transaction::{ClientInput, ClientOutput, TransactionBuilder};
use fedimint_core::core::{Decoder, KeyPair, OperationId};
use fedimint_core::db::Database;
use fedimint_core::module::{ApiVersion, ModuleCommon, MultiApiVersion, TransactionItemAmount};
use fedimint_core::{apply, async_trait_maybe_send, Amount, OutPoint};
use fedimint_escrow_common::config::EscrowClientConfig;
use fedimint_escrow_common::{
    hash256, ArbiterDecision, EscrowInput, EscrowInputArbiterDecision,
    EscrowInputClamingAfterDispute, EscrowInputDisputing, EscrowInputForClaming, EscrowInputSeller,
    EscrowModuleTypes, EscrowOutput, KIND,
};
use fedimint_escrow_server::EscrowStateMachine;
use rand::{thread_rng, Rng};
use secp256k1::PublicKey;

pub mod cli;

/// The escrow client module
#[derive(Debug)]
pub struct EscrowClientModule {
    cfg: EscrowClientConfig,
    key: KeyPair,
    notifier: ModuleNotifier<EscrowStateMachine>,
    client_ctx: ClientContext<Self>,
    db: Database,
}

/// Data needed by the state machine as context
#[derive(Debug, Clone)]
pub struct EscrowClientContext {
    pub escrow_decoder: Decoder,
}

// escrow module doesn't need local context
impl Context for EscrowClientContext {}

#[apply(async_trait_maybe_send!)]
impl ClientModule for EscrowClientModule {
    type Init = EscrowClientInit;
    type Common = EscrowModuleTypes;
    type Backup = NoModuleBackup;
    type ModuleStateMachineContext = EscrowClientContext;
    type States = EscrowStateMachine;

    fn context(&self) -> Self::ModuleStateMachineContext {
        EscrowClientContext {
            escrow_decoder: self.decoder(),
        }
    }

    // conveys the monetary value of escrow input
    fn input_amount(
        &self,
        input: &<Self::Common as ModuleCommon>::Input,
    ) -> Option<TransactionItemAmount> {
        Some(TransactionItemAmount {
            amount: input.amount,
            fee: Amount::ZERO, // seller does not need to pay any fee to get the ecash
        })
    }

    // conveys to the transaction the monetary value of escrow output so as to burn
    // the equivalent ecash
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
    /// Handles the buyer transaction and sends the transaction to the
    /// federation for escrow command
    pub async fn create_escrow(
        &self,
        amount: Amount,
        seller_pubkey: PublicKey,
        arbiter_pubkey: PublicKey,
        escrow_id: String,
        secret_code_hash: String,
    ) -> anyhow::Result<(OperationId, OutPoint)> {
        let operation_id = OperationId(thread_rng().gen());

        let output = EscrowOutput {
            amount,
            buyer_pubkey: self.key.public_key(),
            seller_pubkey,
            arbiter_pubkey,
            escrow_id,
            secret_code_hash,
        };

        // buyer gets statemachine as an asset to track the ecash issued!

        // Build and send tx to the fed by underfunding the transaction
        // The transaction builder will select the necessary e-cash notes with mint
        // output to cover the output amount and create the corresponding inputs itself
        let tx = TransactionBuilder::new().with_output(self.client_ctx.make_client_output(output));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        let tx_subscription = self.client_ctx.transaction_updates(operation_id).await;

        tx_subscription
            .await_tx_accepted(txid)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok((operation_id, change[0]))
    }

    /// Handles the seller transaction and sends the transaction to the
    /// federation for EscrowClaim command
    pub async fn claim_escrow(
        &self,
        escrow_id: String,
        amount: Amount,
        secret_code: String,
    ) -> anyhow::Result<()> {
        // make an api call to server db and get the secret code hash and state of
        // escrow, and then verify it
        let response: [u8; 32] = self
            .client_ctx
            .api()
            .request(GET_MODULE_INFO, escrow_id)
            .await?;
        if response.state == EscrowStates::Disputed {
            return Err(EscrowError::EscrowDisputed);
        }
        if response.state != EscrowState::WaitingforSellerToClaim || EscrowState::Open {
            return Err(EscrowError::ArbiterNotDecided);
        }
        let operation_id = OperationId(thread_rng().gen());
        // Transfer ecash to seller by overfunding the transaction
        // Create input using the buyer account
        let input = EscrowInputForClaming {
            amount,
            secret_code: secret_code,
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx = TransactionBuilder::new().with_input(self.client_ctx.make_client_input(input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        let tx_subscription = self.client_ctx.transaction_updates(operation_id).await;

        tx_subscription
            .await_tx_accepted(txid)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(())
    }

    /// Handles the claiming of transaction and sends the transaction to the
    /// federation
    pub async fn buyer_claim(&self, escrow_id: String, amount: Amount) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());
        // Transfer ecash back to buyer by underfunding the transaction
        let input = EscrowInputClamingAfterDispute { amount };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx = TransactionBuilder::new().with_input(self.client_ctx.make_client_input(input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        let tx_subscription = self.client_ctx.transaction_updates(operation_id).await;

        tx_subscription
            .await_tx_accepted(txid)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(())
    }

    /// Handles the initiate dispute transaction and sends the transaction to
    /// the federation for EscrowDispute command
    pub async fn initiate_dispute(
        &self,
        escrow_id: String,
        arbiter_fee: Amount,
    ) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());

        let escrow_info: ModuleInfo = self
            .client_ctx
            .api()
            .request(GET_MODULE_INFO, escrow_id.clone())
            .await?;

        let input = EscrowInputDisputing {
            amount: arbiter_fee,
            disputer: self.key.public_key(), // the public key of the person who is disputing
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx = TransactionBuilder::new().with_input(self.client_ctx.make_client_input(input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        let tx_subscription = self.client_ctx.transaction_updates(operation_id).await;

        tx_subscription
            .await_tx_accepted(txid)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(())
    }

    /// Handles the arbiter transaction and sends the transaction to the
    /// federation for EscrowArbiterDecision command
    pub async fn arbiter_decision(
        &self,
        escrow_id: String,
        decision: String,
        signature: String,
        signed_message: String,
    ) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());

        let arbiter_decision = match decision.to_lowercase().as_str() {
            "buyer" => ArbiterDecision::BuyerWins,
            "seller" => ArbiterDecision::SellerWins,
            _ => return Err(EscrowError::InvalidArbiterDecision),
        };

        // Transfer ecash back to buyer by underfunding the transaction
        let input = EscrowInputArbiterDecision {
            amount: Amount::ZERO,
            arbiter_decision,
            signed_message,
            signature,
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx = TransactionBuilder::new().with_input(self.client_ctx.make_client_input(input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (txid, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        let tx_subscription = self.client_ctx.transaction_updates(operation_id).await;

        tx_subscription
            .await_tx_accepted(txid)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(())
    }

    // /// Request the federation prints money for us for using in test
    // pub async fn print_money(&self, amount: Amount) ->
    // anyhow::Result<(OperationId, OutPoint)> {
    //     self.print_using_account(amount, fed_key_pair()).await
    // }

    // /// Use a broken printer in test to print a liability instead of money
    // /// If the federation is honest, should always fail
    // pub async fn print_liability(&self, amount: Amount) ->
    // anyhow::Result<(OperationId, OutPoint)> {
    //     self.print_using_account(amount, broken_fed_key_pair())
    //         .await
    // }
}

/// The escrow client module initializer
#[derive(Debug, Clone)]
pub struct EscrowClientInit;

/// Generates the client module
#[apply(async_trait_maybe_send!)]
impl ClientModuleInit for EscrowClientInit {
    type Module = EscrowClientModule;

    fn supported_api_versions(&self) -> MultiApiVersion {
        MultiApiVersion::try_from_iter([ApiVersion { major: 0, minor: 0 }])
            .expect("no version conflicts")
    }

    async fn init(&self, args: &ClientModuleInitArgs<Self>) -> anyhow::Result<Self::Module> {
        Ok(EscrowClientModule {
            cfg: args.cfg().clone(),
            key: args
                .module_root_secret()
                .clone()
                .to_secp_key(&Secp256k1::new()),
            notifier: args.notifier().clone(),
            client_ctx: args.context(),
            db: args.db().clone(),
        })
    }
}
