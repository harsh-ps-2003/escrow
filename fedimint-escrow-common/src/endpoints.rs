use std::time::SystemTime;

use fedimint_core::Amount;
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};

use super::EscrowStates;

// get escrow information in the client side
pub const GET_MODULE_INFO: &str = "get_module_info";

/// EscrowInfo is the response to the GET_MODULE_INFO request
#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
pub struct EscrowInfo {
    pub buyer_pubkey: PublicKey,
    pub seller_pubkey: PublicKey,
    pub arbiter_pubkey: PublicKey,
    pub amount: Amount,
    pub secret_code_hash: String,
    pub state: EscrowStates,
    pub max_arbiter_fee: Amount,
    pub created_at: SystemTime,
}
