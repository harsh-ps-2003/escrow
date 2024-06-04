use std::fmt;

use config::EscrowClientConfig;
use fedimint_core::core::{Decoder, ModuleInstanceId, ModuleKind};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::module::{CommonModuleInit, ModuleCommon, ModuleConsensusVersion};
use fedimint_core::{plugin_types_trait_impl_common, Amount};
use secp256k1::{KeyPair, PublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// Common contains types shared by both the client and server
pub mod config;

/// Unique name for this module
pub const KIND: ModuleKind = ModuleKind::from_static_str("escrow");

/// Modules are non-compatible with older versions
pub const CONSENSUS_VERSION: ModuleConsensusVersion = ModuleConsensusVersion::new(0, 0);

pub const CODE: String = Uuid::new_v4().to_string();

/// Non-transaction items that will be submitted to consensus
/// The Fedimint txn is the only thing that requires consensus from guardians,
/// other than this we are not proposing any changes.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Encodable, Decodable)]
pub struct EscrowConsensusItem;

/*
The finalize_and_submit_transaction_inner function is responsible for finalizing the transaction and preparing it for submission to the federation.
The actual submission to the federation, which would involve adding the transaction to the guardians' database, is handled by the state machine that is created and managed by the executor.
So cant commit values to federation (guardians) DB directly, will need escrowoutput for it most probably!
If we use escrow output, we still want to use mint output in it, so process_output will have to be involved, but wont it be simple copy pasting from mintoutput's process_output?
*/

/// Input for a Fedimint transaction
// #[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
// pub struct EscrowInput {
//     pub amount: Amount,
// }

/// Output for a Fedimint transaction
// #[derive(Debug, Clone, Eq, PartialEq, Hash, Deserialize, Serialize, Encodable, Decodable)]
// pub struct EscrowOutput {
//     pub amount: Amount,
//     pub buyer: KeyPair,
//     pub seller: KeyPair,
//     pub arbiter: PublicKey,
// }

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

// /// A special key that creates assets for a test/example
// const FED_SECRET_PHRASE: &str = "Money printer go brrr...........";

// const BROKEN_FED_SECRET_PHRASE: &str = "Money printer go <boom>........!";

// pub fn fed_public_key() -> PublicKey {
//     fed_key_pair().public_key()
// }

// pub fn fed_key_pair() -> KeyPair {
//     KeyPair::from_seckey_slice(&Secp256k1::new(),
// FED_SECRET_PHRASE.as_bytes()).expect("32 bytes") }

// pub fn broken_fed_public_key() -> PublicKey {
//     broken_fed_key_pair().public_key()
// }

// // Like fed, but with a broken accounting
// pub fn broken_fed_key_pair() -> KeyPair {
//     KeyPair::from_seckey_slice(&Secp256k1::new(),
// BROKEN_FED_SECRET_PHRASE.as_bytes())         .expect("32 bytes")
// }
