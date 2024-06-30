use fedimint_client::sm::{DynState, State, StateTransition};
use fedimint_client::DynGlobalClientContext;
use fedimint_core::core::{IntoDynInstance, ModuleInstanceId, OperationId};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::TransactionId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The state machine for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Decodable, Encodable)]
pub enum EscrowStateMachine {
    Open(String, TransactionId),
    ResolvedWithoutDispute(String, TransactionId), // arbiter gets no fee
    ResolvedWithDispute(String, TransactionId),    // arbiter gets the fee
    Disputed(String, TransactionId),
}

impl State for EscrowStateMachine {
    type ModuleContext = EscrowClientContext;

    fn transitions(
        &self,
        context: &Self::ModuleContext,
        global_context: &DynGlobalClientContext,
    ) -> Vec<StateTransition<Self>> {
        // State transition logic using lib.rs
        // when the transaction by buyer is successful, state is open

        match self.clone() {
            EscrowStateMachine::Open(escrow_id, txid) => vec![StateTransition::new(
                // will need transaction updates.
                await_tx_accepted(global_context.clone(), txid),
                move |dbtx, res, _state: Self| match res {
                    // client writes in own db, that txn completed after server side has already
                    // written it!
                    Ok(_) => Box::pin(async move {
                        EscrowStateMachine::ResolvedWithoutDispute(escrow_id, txid)
                    }),
                    Err(_) => {
                        Box::pin(async move { EscrowStateMachine::Disputed(escrow_id, txid) })
                    }
                },
            )],
            EscrowStateMachine::ResolvedWithoutDispute(escrow_id, txid) => vec![],
            EscrowStateMachine::ResolvedWithDispute(escrow_id, txid) => vec![],
            EscrowStateMachine::Disputed(escrow_id, txid) => vec![],
        }
    }

    // TODO: escrow_id shouldn't be mapped to operation_id as as escrow_id is going
    // to remain same, while for each transaction operation_id will differ
    fn operation_id(&self) -> OperationId {
        match self {
            EscrowStateMachine::Open(escrow_id, txid) => *txid,
            EscrowStateMachine::ResolvedWithoutDispute(escrow_id, txid) => *txid,
            EscrowStateMachine::ResolvedWithDispute(escrow_id, txid) => *txid,
            EscrowStateMachine::Disputed(escrow_id, txid) => *txid,
        }
    }
}

// TODO: Boiler-plate
impl IntoDynInstance for EscrowStateMachine {
    type DynType = DynState;

    fn into_dyn(self, instance_id: ModuleInstanceId) -> Self::DynType {
        DynState::from_typed(instance_id, self)
    }
}

/// The errors for the escrow module
#[derive(Error, Debug, Serialize, Deserialize, Encodable, Decodable, Clone, Eq, PartialEq)]
pub enum EscrowError {
    #[error("Escrow not found")]
    EscrowNotFound,
    #[error("Retreat time not passed")]
    RetreatTimeNotPassed,
    #[error("Escrow is disputed and cannot be claimed")]
    EscrowDisputed,
    #[error("Invalid secret code")]
    InvalidSecretCode,
    #[error("Escrow is not disputed, thus arbiter cannot decide the ecash to be given to buyer or seller")]
    EscrowNotDisputed,
    #[error("You are not the Arbiter!")]
    ArbiterNotMatched,
    #[error("Arbiter has not decided the ecash to be given to buyer or seller yet!")]
    ArbiterNotDecided,
    #[error("Invalid arbiter state")]
    InvalidArbiterState,
    #[error("Invalid state for initiating dispute")]
    InvalidStateForInitiatingDispute,
    #[error("Invalid state for claiming escrow")]
    InvalidStateForClaimingEscrow,
    #[error("Unauthorized to dispute this escrow")]
    UnauthorizedToDispute,
    #[error("Invalid state for arbiter decision")]
    InvalidStateForArbiterDecision,
    #[error("Invalid arbiter signature")]
    InvalidArbiter,
    #[error("Invalid arbiter decision, either the winner can be the buyer or the seller")]
    InvalidArbiterDecision,
}
