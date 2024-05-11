use std::time::Duration;

use fedimint_client::sm::{DynState, State, StateTransition};
use fedimint_client::DynGlobalClientContext;
use fedimint_core::core::{Decoder, IntoDynInstance, ModuleInstanceId, OperationId};
use fedimint_core::db::{DatabaseTransaction, IDatabaseTransactionOpsCoreTyped};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::task::sleep;
use fedimint_core::{Amount, OutPoint, TransactionId};
use fedimint_escrow_common::EscrowOutputOutcome;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use crate::db::EscrowClientFundsKeyV1;
use crate::{get_funds, EscrowClientContext};

const RETRY_DELAY: Duration = Duration::from_secs(1);

/// Tracks a transaction
#[derive(Debug, Clone, Eq, PartialEq, Hash, Decodable, Encodable)]
pub enum EscrowStateMachine {
    Input(Amount, TransactionId, OperationId),
    Output(Amount, TransactionId, OperationId),
    InputDone(OperationId),
    OutputDone(Amount, OperationId),
    Refund(OperationId),
    Unreachable(OperationId, Amount),
}

impl State for EscrowStateMachine {
    type ModuleContext = EscrowClientContext;

    fn transitions(
        &self,
        context: &Self::ModuleContext,
        global_context: &DynGlobalClientContext,
    ) -> Vec<StateTransition<Self>> {
        match self.clone() {
            EscrowStateMachine::Input(amount, txid, id) => vec![StateTransition::new(
                await_tx_accepted(global_context.clone(), txid),
                move |dbtx, res, _state: Self| match res {
                    // accepted, we are done
                    Ok(_) => Box::pin(async move { EscrowStateMachine::InputDone(id) }),
                    // tx rejected, we refund ourselves
                    Err(_) => Box::pin(async move {
                        add_funds(amount, dbtx.module_tx()).await;
                        EscrowStateMachine::Refund(id)
                    }),
                },
            )],
            EscrowStateMachine::Output(amount, txid, id) => vec![StateTransition::new(
                await_escrow_output_outcome(
                    global_context.clone(),
                    OutPoint { txid, out_idx: 0 },
                    context.escrow_decoder.clone(),
                ),
                move |dbtx, res, _state: Self| match res {
                    // output accepted, add funds
                    Ok(_) => Box::pin(async move {
                        add_funds(amount, dbtx.module_tx()).await;
                        EscrowStateMachine::OutputDone(amount, id)
                    }),
                    // output rejected, do not add funds
                    Err(_) => Box::pin(async move { EscrowStateMachine::Refund(id) }),
                },
            )],
            EscrowStateMachine::InputDone(_) => vec![],
            EscrowStateMachine::OutputDone(_, _) => vec![],
            EscrowStateMachine::Refund(_) => vec![],
            EscrowStateMachine::Unreachable(_, _) => vec![],
        }
    }

    fn operation_id(&self) -> OperationId {
        match self {
            EscrowStateMachine::Input(_, _, id) => *id,
            EscrowStateMachine::Output(_, _, id) => *id,
            EscrowStateMachine::InputDone(id) => *id,
            EscrowStateMachine::OutputDone(_, id) => *id,
            EscrowStateMachine::Refund(id) => *id,
            EscrowStateMachine::Unreachable(id, _) => *id,
        }
    }
}

async fn add_funds(amount: Amount, mut dbtx: DatabaseTransaction<'_>) {
    let funds = get_funds(&mut dbtx).await + amount;
    dbtx.insert_entry(&EscrowClientFundsKeyV1, &funds).await;
}

// TODO: Boiler-plate, should return OutputOutcome
async fn await_tx_accepted(
    context: DynGlobalClientContext,
    txid: TransactionId,
) -> Result<(), String> {
    context.await_tx_accepted(txid).await
}

async fn await_escrow_output_outcome(
    global_context: DynGlobalClientContext,
    outpoint: OutPoint,
    module_decoder: Decoder,
) -> Result<(), EscrowError> {
    loop {
        match global_context
            .api()
            .await_output_outcome::<EscrowOutputOutcome>(
                outpoint,
                Duration::from_millis(i32::MAX as u64),
                &module_decoder,
            )
            .await
        {
            Ok(_) => {
                return Ok(());
            }
            Err(e) if e.is_rejected() => {
                return Err(EscrowError::EscrowInternalError);
            }
            Err(e) => {
                e.report_if_important();
                debug!(error = %e, "Awaiting output outcome failed, retrying");
            }
        }
        sleep(RETRY_DELAY).await;
    }
}

// TODO: Boiler-plate
impl IntoDynInstance for EscrowStateMachine {
    type DynType = DynState;

    fn into_dyn(self, instance_id: ModuleInstanceId) -> Self::DynType {
        DynState::from_typed(instance_id, self)
    }
}

#[derive(Error, Debug, Serialize, Deserialize, Encodable, Decodable, Clone, Eq, PartialEq)]
pub enum EscrowError {
    #[error("Escrow module had an internal error")]
    EscrowInternalError,
}
