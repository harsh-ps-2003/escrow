use fedimint_core::core::ModuleInstanceId;
use fedimint_core::db::{DatabaseTransaction, DatabaseValue, IDatabaseTransactionOpsCoreTyped};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::registry::ModuleDecoderRegistry;
use fedimint_core::sm::DynState;
use fedimint_core::{impl_db_record, Amount};
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use strum_macros::EnumIter;

use super::states::EscrowStates;

/// The key prefix for the database
#[repr(u8)]
#[derive(Clone, Debug, EnumIter)]
pub enum DbKeyPrefix {
    Escrow = 0x04,
}

impl std::fmt::Display for DbKeyPrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// The key structure using a UUID
#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
pub struct EscrowKey {
    pub escrow_id: String,
}

/// The structure for the database record
#[derive(Debug, Serialize, Deserialize)]
pub struct EscrowValue {
    pub buyer_pubkey: PublicKey,
    pub seller_pubkey: PublicKey,
    pub arbiter_pubkey: PublicKey,
    pub amount: Amount,
    pub code_hash: [u8; 32],
    pub state: EscrowStates,
    pub created_at: u64,
    pub retreat_duration: u64,
}

/// Implement database record creation and lookup
impl_db_record!(
    key = EscrowKey,
    value = EscrowValue,
    db_prefix = DbKeyPrefix::Escrow,
);
