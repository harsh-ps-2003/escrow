use std::fmt;

use config::EscrowClientConfig;
use fedimint_core::core::{Decoder, ModuleInstanceId, ModuleKind};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::{CommonModuleInit, ModuleCommon, ModuleConsensusVersion};
use fedimint_core::{plugin_types_trait_impl_common, Amount};
use hex;
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

// Common contains types shared by both the client and server
pub mod config;

/// Unique name for this module
pub const KIND: ModuleKind = ModuleKind::from_static_str("escrow");

/// Modules are non-compatible with older versions
pub const CONSENSUS_VERSION: ModuleConsensusVersion = ModuleConsensusVersion::new(0, 0);

/// Non-transaction items that will be submitted to consensus
/// The Fedimint txn is the only thing that requires consensus from guardians,
/// other than this we are not proposing any changes.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Encodable, Decodable)]
pub struct EscrowConsensusItem;

#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub enum EscrowAction {
    Claim,
    Dispute,
    Retreat,
}

/// The input for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct EscrowInput {
    pub amount: Amount,
    pub secret_code: Option<String>,
    pub action: EscrowAction,
    pub arbiter_state: Option<String>,
}

/// The output for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct EscrowOutput {
    pub amount: Amount,
    pub buyer_pubkey: PublicKey,
    pub seller_pubkey: PublicKey,
    pub arbiter_pubkey: PublicKey,
    pub escrow_id: String,
    pub retreat_duration: u64,
}

/// Errors that might be returned by the server when the buyer awaits guardians
/// that the requested amount is burned
#[derive(Debug, Clone, Eq, PartialEq, Hash, Error, Encodable, Decodable)]
pub enum EscrowInputError {}

/// Errors that might be returned by the server
#[derive(Debug, Clone, Eq, PartialEq, Hash, Error, Encodable, Decodable)]
pub enum EscrowOutputError {}

/// Contains the types defined above
pub struct EscrowModuleTypes;

// Wire together the types for this module
plugin_types_trait_impl_common!(
    EscrowModuleTypes,
    EscrowClientConfig,
    EscrowInput,
    EscrowOutput,
    EscrowConsensusItem,
    EscrowInputError,
    EscrowOutputError,
);

/// The common initializer for the escrow module
#[derive(Debug)]
pub struct EscrowCommonInit;

impl CommonModuleInit for EscrowCommonInit {
    const CONSENSUS_VERSION: ModuleConsensusVersion = CONSENSUS_VERSION;
    const KIND: ModuleKind = KIND;

    type ClientConfig = EscrowClientConfig;

    fn decoder() -> Decoder {
        EscrowModuleTypes::decoder_builder().build()
    }
}

impl fmt::Display for EscrowClientConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EscrowClientConfig")
    }
}

impl fmt::Display for EscrowInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EscrowInput: {}", self.amount)
    }
}

impl fmt::Display for EscrowOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EscrowOutput: {}", self.amount)
    }
}

/// Hashes the value using SHA256
pub fn hash256(value: String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}
