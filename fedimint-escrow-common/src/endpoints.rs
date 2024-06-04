use serde::{Deserialize, Serialize};
use uuid::Uuid;

// get escrow information in the client side
pub const GET_MODULE_INFO: &str = "get_module_info";

#[derive(Debug, Serialize, Deserialize)]
pub struct GetModuleInfoRequest {
    pub escrow_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub buyer: PublicKey,
    pub seller: PublicKey,
    pub arbiter: PublicKey,
    pub amount: Amount,
    pub code_hash: [u8; 32],
}
