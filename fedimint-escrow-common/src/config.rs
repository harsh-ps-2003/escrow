use fedimint_core::core::ModuleKind;
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::{plugin_types_trait_impl_config, Amount};
use serde::{Deserialize, Serialize};

use crate::EscrowCommonInit;

/// Parameters necessary to generate this module's configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowGenParams {
    pub local: EscrowGenParamsLocal,
    pub consensus: EscrowGenParamsConsensus,
}

/// Local parameters for config generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowGenParamsLocal;

/// Consensus parameters for config generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowGenParamsConsensus {
    pub tx_fee: Amount,
}

impl Default for EscrowGenParams {
    fn default() -> Self {
        Self {
            local: EscrowGenParamsLocal,
            consensus: EscrowGenParamsConsensus {
                tx_fee: Amount::ZERO,
            },
        }
    }
}

/// Contains all the configuration for the server
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EscrowConfig {
    pub local: EscrowConfigLocal,
    pub private: EscrowConfigPrivate,
    pub consensus: EscrowConfigConsensus,
}

/// Contains all the configuration for the client
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encodable, Decodable, Hash)]
pub struct EscrowClientConfig {
    /// Accessible to clients
    pub tx_fee: Amount,
}

/// Locally unencrypted config unique to each member
#[derive(Clone, Debug, Serialize, Deserialize, Decodable, Encodable)]
pub struct EscrowConfigLocal;

/// Will be the same for every federation member
#[derive(Clone, Debug, Serialize, Deserialize, Decodable, Encodable)]
pub struct EscrowConfigConsensus {
    /// Will be the same for all peers
    pub tx_fee: Amount,
}

/// Will be encrypted and not shared such as private key material
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EscrowConfigPrivate;

// Wire together the configs for this module
plugin_types_trait_impl_config!(
    EscrowCommonInit,
    EscrowGenParams,
    EscrowGenParamsLocal,
    EscrowGenParamsConsensus,
    EscrowConfig,
    EscrowConfigLocal,
    EscrowConfigPrivate,
    EscrowConfigConsensus,
    EscrowClientConfig
);
