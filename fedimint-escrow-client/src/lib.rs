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
use fedimint_escrow_common::{EscrowInput, EscrowModuleTypes, EscrowOutput, Note, KIND};
use fedimint_escrow_server::states::EscrowStateMachine;
use secp256k1::{PublicKey, Secp256k1};
use uuid::Uuid;

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
            fee: Amount::ZERO, // seller does not need to pay any fee
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
            fee: self.cfg.deposit_fee, //fee is required to use the escrow service
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

// attach ecash to the transaction and submit it to federation
impl EscrowClientModule {
    pub async fn buyer_txn(
        &self,
        amount: Amount,
        buyer: PublicKey,
        seller: PublicKey,
        arbiter: PublicKey,
    ) -> anyhow::Result<(OperationId, OutPoint)> {
        let operation_id = OperationId(thread_rng().gen());
        let escrow_id = Uuid::new_v4();

        // Create input using the buyer account
        let input = ClientInput {
            input: EscrowInput {
                amount: Amount::ZERO,
                note: Note::new(),
            },
            keys: vec![self.key],
            state_machines: Arc::new(move |_, _| Vec::<EscrowStateMachine>::new()),
        };

        // Create output using the seller account
        let output = ClientOutput {
            output: EscrowOutput {
                amount,
                buyer,
                seller,
                arbiter,
            },
            state_machines: Arc::new(move |_, _| Vec::<EscrowStateMachine>::new()),
        };

        // Build and send tx to the fed by underfunding the transaction
        // Will output to mint module
        let tx = TransactionBuilder::new()
            .with_input(self.client_ctx.make_client_input(input))
            .with_output(self.client_ctx.make_client_output(output));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (_, change) = self
            .client_ctx
            .finalize_and_submit_transaction(op_id, KIND.as_str(), outpoint, tx)
            .await?;

        self.change_state_to_open(escrow_id).await?;

        Ok((operation_id, change[0], escrow_id))
    }

    pub async fn seller_txn(&self, escrow_id: Uuid, secret_code: String) -> anyhow::Result<()> {
        let operation_id = OperationId(thread_rng().gen());
        // Transfer ecash to seller by overfunding the transaction
        // Create input using the buyer account
        let input = ClientInput {
            input: EscrowInput {
                amount,
                note: Note::new(),
            },
            keys: vec![], // seller keypair?
            state_machines: Arc::new(move |_, _| Vec::<EscrowStateMachine>::new()),
        };

        // Create output using the seller account
        let output = ClientOutput {
            output: EscrowOutput {
                amount: Amount::ZERO,
                buyer,
                seller,
                arbiter,
            },
            state_machines: Arc::new(move |_, _| Vec::<EscrowStateMachine>::new()),
        };

        // Build and send tx to the fed
        let tx = TransactionBuilder::new()
            .with_input(self.client_ctx.make_client_input(input))
            .with_output(self.client_ctx.make_client_output(output));
        let outpoint = |txid, _| OutPoint { txid, out_idx: 0 };
        let (_, change) = self
            .client_ctx
            .finalize_and_submit_transaction(operation_id, KIND.as_str(), outpoint, tx)
            .await?;
        self.change_state_to_resolved(escrow_id).await?;
        Ok(())
    }

    async fn change_state_to_open(&self, escrow_id: Uuid) -> anyhow::Result<()> {
        let dbtx = self.client_ctx.db().begin_transaction().await?;
        let new_state = EscrowStateMachine::Open(escrow_id);
        dbtx.insert_entry(
            &EscrowKey {
                uuid: escrow_id.to_string(),
            },
            &new_state,
        )
        .await?;
        dbtx.commit().await?;
        Ok(())
    }

    async fn change_state_to_resolved(&self, escrow_id: Uuid) -> anyhow::Result<()> {
        let dbtx = self.client_ctx.db().begin_transaction().await?;
        let escrow_state: EscrowStateMachine = dbtx
            .get_value(&EscrowKey {
                uuid: escrow_id.to_string(),
            })
            .await?
            .ok_or_else(|| anyhow::anyhow!("Escrow not found"))?;

        match escrow_state {
            EscrowStateMachine::Open(_) => {
                // Change state to ResolvedWithoutDispute
                let new_state = EscrowStateMachine::ResolvedWithoutDispute(escrow_id);
                dbtx.insert_entry(
                    &EscrowKey {
                        uuid: escrow_id.to_string(),
                    },
                    &new_state,
                )
                .await?;
            }
            EscrowStateMachine::Disputed(_) => {
                // Change state to ResolvedWithDispute
                let new_state = EscrowStateMachine::ResolvedWithDispute(escrow_id);
                dbtx.insert_entry(
                    &EscrowKey {
                        uuid: escrow_id.to_string(),
                    },
                    &new_state,
                )
                .await?;
            }
            _ => return Err(anyhow!("Invalid state for claiming escrow")),
        }
        dbtx.commit().await?;
        Ok(())
    }

    pub async fn initiate_dispute(&self, escrow_id: Uuid) -> anyhow::Result<()> {
        // Call the arbiter (this could be a network call, a message, etc.)?
        self.call_arbiter(escrow_id).await?;

        // Change the state to Disputed
        let dbtx = self.client_ctx.db().begin_transaction().await?;
        let new_state = EscrowStateMachine::Disputed(escrow_id);
        dbtx.insert_entry(
            &EscrowKey {
                uuid: escrow_id.to_string(),
            },
            &new_state,
        )
        .await?;
        dbtx.commit().await?;

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
