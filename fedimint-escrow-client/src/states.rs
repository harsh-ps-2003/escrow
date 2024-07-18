use std::pin::Pin;

use anyhow::Error;
use fedimint_client::sm::{Context, DynState, State, StateTransition};
use fedimint_client::DynGlobalClientContext;
use fedimint_core::core::{Decoder, IntoDynInstance, ModuleInstanceId, OperationId};
use fedimint_core::encoding::{Decodable, Encodable};
use futures::Future;
use rand::{thread_rng, Rng};

#[derive(Debug, Clone, Eq, PartialEq, Hash, Decodable, Encodable)]
pub struct EscrowStateMachine;

/// Data needed by the state machine as context
#[derive(Debug, Clone)]
pub struct EscrowClientContext {
    pub escrow_decoder: Decoder,
}

// escrow module doesn't need local context
impl Context for EscrowClientContext {}

impl State for EscrowStateMachine {
    type ModuleContext = EscrowClientContext;
    fn transitions(
        &self,
        _context: &Self::ModuleContext,
        _global_context: &DynGlobalClientContext,
    ) -> Vec<StateTransition<Self>> {
        // transition to the same state on the client side
        vec![StateTransition::new(
            async { () },
            |_db_tx, _, old_state: Self| Box::pin(async move { old_state }),
        )]
    }

    fn operation_id(&self) -> OperationId {
        OperationId(thread_rng().gen())
    }
}

impl IntoDynInstance for EscrowStateMachine {
    type DynType = DynState;

    fn into_dyn(self, instance_id: ModuleInstanceId) -> Self::DynType {
        DynState::from_typed(instance_id, self)
    }
}
