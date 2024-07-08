pub mod endpoints;

use std::fmt;

use config::EscrowClientConfig;
use fedimint_core::core::{Decoder, ModuleInstanceId, ModuleKind};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::{CommonModuleInit, ModuleCommon, ModuleConsensusVersion};
use fedimint_core::{plugin_types_trait_impl_common, Amount};
use hex;
use secp256k1::schnorr::Signature;
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

impl std::fmt::Display for EscrowConsensusItem {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unimplemented!()
    }
}

/// The states for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Decodable, Encodable, Serialize, Deserialize)]
pub enum EscrowStates {
    /// the escrow is created and not claimed by buyer or seller, thus its open
    Open,
    /// the escrow is resolved without dispute
    ResolvedWithoutDispute,
    /// the escrow is resolved with dispute
    ResolvedWithDispute,
    /// the escrow is disputed by buyer
    DisputedByBuyer,
    /// the escrow is disputed by seller
    DisputedBySeller,
    /// buyer has won the dispute and has to claim the escrow
    WaitingforBuyerToClaim,
    /// seller has won the dispute and has to claim the escrow
    WaitingforSellerToClaim,
}

/// The disputer in the escrow, can either be buyer or the seller
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Disputer {
    Buyer,
    Seller,
}

/// The arbiter decision on who won the dispute, either the buyer or the seller
#[derive(Debug, Clone, Eq, PartialEq, Hash, Encodable, Decodable)]
pub enum ArbiterDecision {
    BuyerWins,
    SellerWins,
}

/// The input for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Encodable, Decodable)]
pub enum EscrowInput {
    /// The input when seller is claiming the escrow without any dispute
    ClamingWithoutDispute(EscrowInputClamingWithoutDispute),
    /// The input when buyer or seller is disputing the escrow
    Disputing(EscrowInputDisputing),
    /// The input when buyer or seller is claiming the escrow after the dispute
    ClaimingAfterDispute(EscrowInputClaimingAfterDispute),
    /// The input when arbiter is deciding who won the dispute
    ArbiterDecision(EscrowInputArbiterDecision),
}
/// The input for the escrow module when the seller is claiming the escrow using
/// the secret code
#[derive(Debug, Clone, Eq, PartialEq, Hash, Encodable, Decodable)]
pub struct EscrowInputClamingWithoutDispute {
    pub amount: Amount,
    pub escrow_id: String,
    pub secret_code: String,
    pub hashed_message: [u8; 32],
    pub signature: Signature,
}

/// The input for the escrow module when the buyer or seller is disputing the
/// escrow
#[derive(Debug, Clone, Eq, PartialEq, Hash, Encodable, Decodable)]
pub struct EscrowInputDisputing {
    pub escrow_id: String,
    pub disputer: PublicKey,
    pub hashed_message: [u8; 32],
    pub signature: Signature,
}

/// The input for the escrow module when the seller or the buyer whosoever in
/// favour arbiter decided is claiming the escrow without using the secret code
#[derive(Debug, Clone, Eq, PartialEq, Hash, Encodable, Decodable)]
pub struct EscrowInputClaimingAfterDispute {
    pub amount: Amount,
    pub escrow_id: String,
    pub hashed_message: [u8; 32],
    pub signature: Signature,
}

/// The input for the escrow module when the seller is claiming the escrow using
/// the secret code
#[derive(Debug, Clone, Eq, PartialEq, Hash, Encodable, Decodable)]
pub struct EscrowInputArbiterDecision {
    pub amount: Amount,
    pub escrow_id: String,
    pub arbiter_decision: ArbiterDecision,
    pub hashed_message: [u8; 32],
    pub signature: Signature,
}

/// The output for the escrow module
#[derive(Debug, Clone, Eq, PartialEq, Hash, Encodable, Decodable)]
pub struct EscrowOutput {
    pub amount: Amount,
    pub buyer_pubkey: PublicKey,
    pub seller_pubkey: PublicKey,
    pub arbiter_pubkey: PublicKey,
    pub escrow_id: String,
    pub secret_code_hash: String,
    pub max_arbiter_fee: Amount,
}

/// Errors that might be returned by the server when the buyer awaits guardians
/// that the requested amount is burned
#[derive(Debug, Clone, Eq, PartialEq, Hash, Error, Encodable, Decodable)]
pub enum EscrowInputError {
    #[error("Invalid secret code")]
    InvalidSecretCode,
    #[error("Escrow is not disputed, thus arbiter cannot decide the ecash to be given to buyer or seller")]
    EscrowNotDisputed,
    #[error("You are not the Arbiter!")]
    ArbiterNotMatched,
    #[error("Invalid arbiter state")]
    InvalidArbiterState,
    #[error("Invalid state for initiating dispute")]
    InvalidStateForInitiatingDispute,
    #[error("Invalid state for claiming escrow")]
    InvalidStateForClaimingEscrow,
    #[error("Unauthorized to dispute this escrow")]
    UnauthorizedToDispute,
    #[error("Invalid state for arbiter decision")]
    InvalidStateForArbiterDecision,
    #[error("Invalid arbiter signature")]
    InvalidArbiter,
    #[error("Invalid max arbiter fee in bps, it should be in range 10 to 1000")]
    InvalidMaxArbiterFeeBps,
    #[error("Invalid seller")]
    InvalidSeller,
    #[error("Invalid buyer")]
    InvalidBuyer,
    #[error("Escrow not found")]
    EscrowNotFound,
    #[error("Invalid public key")]
    InvalidPublicKey(String),
    #[error("Arbiter fee exceeds the maximum allowed")]
    ArbiterFeeExceedsMaximum,
}

/// Errors that might be returned by the server
#[derive(Debug, Clone, Eq, PartialEq, Hash, Error, Encodable, Decodable)]
pub enum EscrowOutputError {}

/// The errors for the escrow module in client side
#[derive(Debug, Clone, Eq, PartialEq, Hash, Error, Encodable, Decodable)]
pub enum EscrowError {
    #[error("Escrow is disputed and cannot be claimed")]
    EscrowDisputed,
    #[error("Arbiter has not decided the ecash to be given to buyer or seller yet!")]
    ArbiterNotDecided,
    #[error("Invalid arbiter decision, either the winner can be the buyer or the seller")]
    InvalidArbiterDecision,
    #[error("Transaction was rejected")]
    TransactionRejected,
    #[error("Escrow not found")]
    EscrowNotFound,
}

impl From<secp256k1::Error> for EscrowInputError {
    fn from(error: secp256k1::Error) -> Self {
        EscrowInputError::InvalidPublicKey(error.to_string())
    }
}

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
    EscrowOutputOutcome,
    EscrowConsensusItem,
    EscrowInputError,
    EscrowOutputError
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
        write!(
            f,
            "EscrowClientConfig {{ deposit_fee: {} }}",
            self.deposit_fee
        )
    }
}

impl fmt::Display for EscrowInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EscrowInput::ClamingWithoutDispute(input) => write!(
                f,
                "EscrowInput::ClamingWithoutDispute {{ amount: {}, secret_code: {} }}",
                input.amount, input.secret_code
            ),
            EscrowInput::Disputing(input) => write!(
                f,
                "EscrowInput::Disputing {{ disputer: {:?} }}",
                input.disputer
            ),
            EscrowInput::ClaimingAfterDispute(input) => write!(
                f,
                "EscrowInput::ClaimingAfterDispute {{ amount: {} }}",
                input.amount
            ),
            EscrowInput::ArbiterDecision(input) => write!(
                f,
                "EscrowInput::ArbiterDecision {{ amount: {}, decision: {:?}, signature: {}}}",
                input.amount,
                input.arbiter_decision,
                hex::encode(input.signature.as_ref()),
            ),
        }
    }
}

impl fmt::Display for EscrowOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "EscrowOutput {{ amount: {}, buyer_pubkey: {:?}, seller_pubkey: {:?}, arbiter_pubkey: {:?}, escrow_id: {}, secret_code_hash: {}, max_arbiter_fee: {} }}",
            self.amount,
            self.buyer_pubkey,
            self.seller_pubkey,
            self.arbiter_pubkey,
            self.escrow_id,
            self.secret_code_hash,
            self.max_arbiter_fee
        )
    }
}

/// Hashes the value using SHA256
pub fn hash256(value: String) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}
