use fedimint_client::sm::{DynState, State, StateTransition};
use fedimint_client::DynGlobalClientContext;
use fedimint_core::core::{IntoDynInstance, ModuleInstanceId, OperationId};
use fedimint_core::encoding::{Decodable, Encodable};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::db::EscrowKey;

/// Tracks a escrow
#[derive(Debug, Clone, Eq, PartialEq, Hash, Decodable, Encodable)]
pub enum EscrowStateMachine {
    Open(escrow_id),
    ResolvedWithoutDispute(escrow_id), // arbiter gets no fee
    ResolvedWithDispute(escrow_id), // arbiter gets the fee
    Disputed(escrow_id),
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
            EscrowStateMachine::Open(escrow_id) => vec![StateTransition::new(
                await_tx_accepted(global_context.clone(), txid),
                move |dbtx, res, _state: Self| match res {
                    Ok(_) => Box::pin(async move { EscrowStateMachine::ResolvedWithoutDispute(escrow_id) }),
                    Err(_) => Box::pin(async move { EscrowStateMachine::Disputed(escrow_id) }),
                },
            )],
            EscrowStateMachine::ResolvedWithoutDispute(escrow_id) => vec![],
            EscrowStateMachine::ResolvedWithDispute(escrow_id) => vec![],
            EscrowStateMachine::Disputed(escrow_id) => vec![],
        }
    }

    fn operation_id(&self) -> OperationId {
        match self {
            EscrowStateMachine::Open(escrow_id) => *escrow_id,
            EscrowStateMachine::ResolvedWithoutDispute(escrow_id) => *escrow_id,
            EscrowStateMachine::ResolvedWithDispute(escrow_id) => *escrow_id,
            EscrowStateMachine::Disputed(escrow_id) => *escrow_id,
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

#[derive(Error, Debug, Serialize, Deserialize, Encodable, Decodable, Clone, Eq, PartialEq)]
pub enum EscrowError {}
