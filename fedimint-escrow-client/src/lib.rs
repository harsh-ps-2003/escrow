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
use fedimint_core::{apply, async_trait_maybe_send, Amount, OperationId, OutPoint};
use fedimint_escrow_common::config::EscrowClientConfig;
use fedimint_escrow_common::{hash256, EscrowInput, EscrowModuleTypes, EscrowOutput, KIND};
use fedimint_escrow_server::states::EscrowStateMachine;
use rand::Rng;
use secp256k1::{PublicKey, Secp256k1};

use crate::cli::CODE;

#[cfg(feature = "cli")]
pub mod cli;

#[derive(Debug)]
pub struct EscrowClientModule {
    cfg: EscrowClientConfig,
    key: KeyPair,
    notifier: ModuleNotifier<EscrowStateMachine>,
    client_ctx: ClientContext<Self>,
    db: Database,
}

/// Data needed by the state machine
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
    // attach ecash to the transaction and submit it to federation
    pub async fn buyer_txn(
        &self,
        amount: Amount,
        buyer: PublicKey,
        seller: PublicKey,
        arbiter: PublicKey,
        retreat_duration: u64,
    ) -> anyhow::Result<(OperationId, OutPoint)> {
        let operation_id = OperationId(thread_rng().gen());
        let escrow_id = hash256(vec![buyer, seller, arbiter, amount.to_string()].concat()); // create escrow id by hashing buyer, seller, arbiter, amount in a ascending
                                                                                            // order

        let output = EscrowOutput {
            amount,
            buyer,
            seller,
            arbiter,
            state: EscrowStates::Open,
            escrow_id,
            retreat_duration,
        };

        // buyer gets statemachine as an asset to track the ecash issued!

        // Build and send tx to the fed by underfunding the transaction
        // The transaction builder will select the necessary e-cash notes with mint
        // output to cover the output amount and create the corresponding inputs itself
        let tx = TransactionBuilder::new().with_output(self.client_ctx.make_client_output(output));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (_, change) = self
            .client_ctx
            .finalize_and_submit_transaction(op_id, KIND.as_str(), outpoint, tx)
            .await?;

        let tx_subscription = self.client_ctx.transaction_updates(op_id).await;

        tx_subscription
            .await_tx_accepted(txid)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok((operation_id, change[0], escrow_id))
    }

    // dont involve consensus until necessary
    // code should not work when arbiter is not involved!
    // when arbiter involved, 2 txns are involved!

    pub async fn seller_txn(
        &self,
        escrow_id: String,
        secret_code: String,
        amount: Amount,
    ) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());
        // Transfer ecash to seller by overfunding the transaction
        // Create input using the buyer account
        let input = EscrowInput {
            amount,
            secret_code: Some(secret_code),
            action: EscrowAction::Claim,
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx = TransactionBuilder::new().with_input(self.client_ctx.make_client_input(input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (_, change) = self
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

    pub async fn retreat_txn(&self, escrow_id: String, amount: Amount) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());
        // Transfer ecash back to buyer by underfunding the transaction
        let input = EscrowInput {
            amount,
            secret_code: None,
            action: EscrowAction::Retreat,
        };

        // Build and send tx to the fed
        // The transaction builder will create mint output to cover the input amount by
        // itself
        let tx = TransactionBuilder::new().with_input(self.client_ctx.make_client_input(input));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (_, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;

        let tx_subscription = self.client_ctx.transaction_updates(op_id).await;

        tx_subscription
            .await_tx_accepted(txid)
            .await
            .map_err(|e| anyhow!(e))?;

        Ok(())
    }

    // think how arbiter will initiate dispute with a txn (input only) of its own
    // and change state
    pub async fn initiate_dispute(&self, escrow_id: String) -> anyhow::Result<()> {
        // TODO : a consensus item submitted via guardian api endpoint? how and why?
        // can dispute be rejected?
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
