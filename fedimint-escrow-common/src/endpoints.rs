use serde::{Deserialize, Serialize};

// get escrow information in the client side
pub const GET_MODULE_INFO: &str = "get_module_info";

pub const GET_SECRET_CODE_HASH: &str = "get_secret_code_hash";

#[derive(Debug, Serialize, Deserialize)]
pub struct GetModuleInfoRequest {
    pub escrow_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub buyer: PublicKey,
    pub seller: PublicKey,
    pub arbiter: PublicKey,
    pub amount: Amount,
    pub code_hash: [u8; 32],
    pub state: EscrowState,
}
