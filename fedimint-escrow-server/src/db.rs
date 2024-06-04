use fedimint_client::sm::DynState;
use fedimint_core::core::ModuleInstanceId;
use fedimint_core::db::{DatabaseTransaction, DatabaseValue, IDatabaseTransactionOpsCoreTyped};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::registry::ModuleDecoderRegistry;
use fedimint_core::{impl_db_record, Amount};
use fedimint_escrow_common::Nonce;
use secp256k1::PublicKey;
use strum_macros::EnumIter;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use sha2::{Digest, Sha256};

// Define the key prefix for the database
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

// Define the key structure using a UUID
#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
pub struct EscrowKey {
    pub uuid: Uuid,
}

// Define the value structure for the database record
#[derive(Debug, Serialize, Deserialize)]
pub struct EscrowValue {
    pub buyer: PublicKey,
    pub seller: PublicKey,
    pub arbiter: PublicKey,
    pub amount: Amount,
    pub code_hash: [u8; 32],
}

// Implement database record creation and lookup
impl_db_record!(
    key = EscrowKey,
    value = EscrowValue,
    db_prefix = DbKeyPrefix::Escrow,
);
