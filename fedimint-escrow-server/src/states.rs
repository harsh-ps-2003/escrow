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

#[derive(Debug, Clone, Eq, PartialEq, Hash, Decodable, Encodable)]
pub enum EscrowStates {
    Created,
    Open,
    Resolved,
    Disputed,
}

impl State for EscrowStateMachine {
    type ModuleContext = EscrowClientContext;

    fn transitions(
        &self,
        context: &Self::ModuleContext,
        global_context: &DynGlobalClientContext,
    ) -> Vec<StateTransition<Self>> {
        match self.clone() {
            EscrowStateMachine::Open(escrow_id) => vec![StateTransition::new(
                await_tx_accepted(global_context.clone(), txid),
                move |dbtx, res, _state: Self| match res {
                    Ok(_) => Box::pin(async move { EscrowStateMachine::ResolvedWithoutDispute(escrow_id) }),
                    Err(_) => Box::pin(async move { EscrowStateMachine::Disputed(escrow_id) }),
                },
            )],
            EscrowStateMachine::ResolvedWithoutDispute(escrow_id) => vec![StateTransition::new(
                async { Ok(()) },
                move |_dbtx, _res, _state: Self| Box::pin(async move { EscrowStateMachine::Closed(escrow_id) }),
            )],
            EscrowStateMachine::ResolvedWithDispute(escrow_id) => vec![StateTransition::new(
                async { Ok(()) },
                move |_dbtx, _res, _state: Self| Box::pin(async move { EscrowStateMachine::Closed(escrow_id) }),
            )],
            EscrowStateMachine::Disputed(escrow_id) => vec![StateTransition::new(
                async { call_arbiter(escrow_id).await },
                move |_dbtx, _res, _state: Self| Box::pin(async move { EscrowStateMachine::Closed(escrow_id) }),
            )],
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
