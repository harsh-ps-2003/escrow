use std::fmt;

use config::EscrowClientConfig;
use fedimint_core::core::{Decoder, ModuleInstanceId, ModuleKind};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::{CommonModuleInit, ModuleCommon, ModuleConsensusVersion};
use fedimint_core::{plugin_types_trait_impl_common, Amount};
use secp256k1::{KeyPair, PublicKey, Secp256k1};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// Common contains types shared by both the client and server

// The client and server configuration
pub mod config;

/// Unique name for this module
pub const KIND: ModuleKind = ModuleKind::from_static_str("escrow");

/// Modules are non-compatible with older versions
pub const CONSENSUS_VERSION: ModuleConsensusVersion = ModuleConsensusVersion::new(0, 0);

/// Non-transaction items that will be submitted to consensus
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Encodable, Decodable)]
pub struct EscrowConsensusItem;

/// Input for a fedimint transaction
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct EscrowInput {
    pub amount: Amount,
    /// Associate the input with a user's pubkey
    pub account: PublicKey,
}

/// Output for a fedimint transaction
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct EscrowOutput {
    pub amount: Amount,
    /// Associate the output with a user's pubkey
    pub account: PublicKey,
}

/// Information needed by a client to update output funds
#[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
pub struct EscrowOutputOutcome(pub Amount, pub PublicKey);

/// Errors that might be returned by the server
#[derive(Debug, Clone, Eq, PartialEq, Hash, Error, Encodable, Decodable)]
pub enum EscrowInputError {
    #[error("Not enough funds")]
    NotEnoughFunds,
}

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
    EscrowOutputOutcome,
    EscrowConsensusItem,
    EscrowInputError,
    EscrowOutputError
);

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
        write!(f, "escrowClientConfig")
    }
}
impl fmt::Display for EscrowInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "escrowInput {}", self.amount)
    }
}

impl fmt::Display for EscrowOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "escrowOutput {}", self.amount)
    }
}

impl fmt::Display for EscrowOutputOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "escrowOutputOutcome")
    }
}

impl fmt::Display for EscrowConsensusItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "escrowConsensusItem")
    }
}

/// A special key that creates assets for a test/example
const FED_SECRET_PHRASE: &str = "Money printer go brrr...........";

const BROKEN_FED_SECRET_PHRASE: &str = "Money printer go <boom>........!";

pub fn fed_public_key() -> PublicKey {
    fed_key_pair().public_key()
}

pub fn fed_key_pair() -> KeyPair {
    KeyPair::from_seckey_slice(&Secp256k1::new(), FED_SECRET_PHRASE.as_bytes()).expect("32 bytes")
}

pub fn broken_fed_public_key() -> PublicKey {
    broken_fed_key_pair().public_key()
}

// Like fed, but with a broken accounting
pub fn broken_fed_key_pair() -> KeyPair {
    KeyPair::from_seckey_slice(&Secp256k1::new(), BROKEN_FED_SECRET_PHRASE.as_bytes())
        .expect("32 bytes")
}
