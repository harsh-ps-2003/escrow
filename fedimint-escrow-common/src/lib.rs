pub mod endpoints;

use std::fmt;

use config::EscrowClientConfig;
use fedimint_core::core::{Decoder, ModuleInstanceId, ModuleKind, OperationId};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::{CommonModuleInit, ModuleCommon, ModuleConsensusVersion};
use fedimint_core::{plugin_types_trait_impl_common, Amount};
use hex;
use secp256k1::{Message, PublicKey, Signature};
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

impl std::fmt::Display for EscrowConsensusItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EscrowConsensusItem")
    }
}

/// The states for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Decodable, Encodable, Serialize, Deserialize)]
pub enum EscrowStates {
    Open,
    ResolvedWithoutDispute,
    ResolvedWithDispute,
    DisputedByBuyer,
    DisputedBySeller,
    WaitingforBuyerToClaim,
    WaitingforSellerToClaim,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Disputer {
    Buyer,
    Seller,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ArbiterDecision {
    BuyerWins,
    SellerWins,
}

pub enum EscrowInput {
    ClamingWithoutDispute(EscrowInputClamingWithoutDispute),
    Disputing(EscrowInputDisputing),
    ClamingAfterDispute(EscrowInputClamingAfterDispute),
    ArbiterDecision(EscrowInputArbiterDecision),
}
/// The input for the escrow module when the seller is claiming the escrow using
/// the secret code
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EscrowInputClamingWithoutDispute {
    pub amount: Amount,
    pub secret_code: String,
}

/// The input for the escrow module when the arbiter needs
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EscrowInputDisputing {
    pub amount: Amount,
    pub disputer: PublicKey,
}

/// The input for the escrow module when the seller or the buyer whosoever in
/// favour arbiter decided is claiming the escrow without using the secret code
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EscrowInputClamingAfterDispute {
    pub amount: Amount,
}

/// The input for the escrow module when the seller is claiming the escrow using
/// the secret code
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EscrowInputArbiterDecision {
    pub amount: Amount,
    pub arbiter_decision: ArbiterDecision,
    pub signature: Signature,
    pub message: Message,
}

/// The output for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct EscrowOutput {
    pub amount: Amount,
    pub buyer_pubkey: PublicKey,
    pub seller_pubkey: PublicKey,
    pub arbiter_pubkey: PublicKey,
    pub escrow_id: String,
    pub secret_code_hash: String,
    pub max_arbiter_fee: Amount,
}

/// The high level state for tracking operations of transactions
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum EscrowOperationState {
    Created,
    Accepted,
    Rejected,
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

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Encodable, Decodable)]
pub enum EscrowOutputOutcome {}

impl std::fmt::Display for EscrowOutputOutcome {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unimplemented!()
    }
}

// Wire together the types for this module
plugin_types_trait_impl_common!(
    EscrowModuleTypes,
    EscrowClientConfig,
    EscrowInput,
    EscrowOutput,
    EscrowConsensusItem,
    EscrowInputError,
    EscrowOutputError,
    EscrowOutputOutcome
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
