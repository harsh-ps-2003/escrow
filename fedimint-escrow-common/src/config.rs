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
    pub deposit_fee: Amount,
    pub max_arbiter_fee_bps: u16,
}

impl Default for EscrowGenParams {
    fn default() -> Self {
        Self {
            local: EscrowGenParamsLocal,
            consensus: EscrowGenParamsConsensus {
                deposit_fee: Amount::ZERO,
                max_arbiter_fee_bps: 0,
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
    pub deposit_fee: Amount,
    pub max_arbiter_fee_bps: u16,
}

impl EscrowClientConfig {
    pub fn limit_max_arbiter_fee_bps(
        &self,
        max_arbiter_fee_bps: u16,
    ) -> Result<u16, anyhow::Error> {
        // the max_arbiter_fee_bps should be in range 10 (0.1%) to 1000 (10%)
        if max_arbiter_fee_bps < 10 || max_arbiter_fee_bps > 1000 {
            Err(anyhow::anyhow!("max_arbiter_fee_bps is out of bounds"))
        } else {
            Ok(max_arbiter_fee_bps)
        }
    }
}

/// Locally unencrypted config unique to each member
#[derive(Clone, Debug, Serialize, Deserialize, Decodable, Encodable)]
pub struct EscrowConfigLocal;

/// Will be the same for every federation member
#[derive(Clone, Debug, Serialize, Deserialize, Decodable, Encodable)]
pub struct EscrowConfigConsensus {
    /// Will be the same for all peers
    pub deposit_fee: Amount,
    pub max_arbiter_fee_bps: u16,
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
