use fedimint_core::core::ModuleKind;
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::{plugin_types_trait_impl_config, Amount};
use serde::{Deserialize, Serialize};

use crate::DummyCommonInit;

/// Parameters necessary to generate this module's configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DummyGenParams {
    pub local: DummyGenParamsLocal,
    pub consensus: DummyGenParamsConsensus,
}

/// Local parameters for config generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DummyGenParamsLocal;

/// Consensus parameters for config generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DummyGenParamsConsensus {
    pub tx_fee: Amount,
}

impl Default for DummyGenParams {
    fn default() -> Self {
        Self {
            local: DummyGenParamsLocal,
            consensus: DummyGenParamsConsensus {
                tx_fee: Amount::ZERO,
            },
        }
    }
}

/// Contains all the configuration for the server
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DummyConfig {
    pub local: DummyConfigLocal,
    pub private: DummyConfigPrivate,
    pub consensus: DummyConfigConsensus,
}

/// Contains all the configuration for the client
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Encodable, Decodable, Hash)]
pub struct DummyClientConfig {
    /// Accessible to clients
    pub tx_fee: Amount,
}

/// Locally unencrypted config unique to each member
#[derive(Clone, Debug, Serialize, Deserialize, Decodable, Encodable)]
pub struct DummyConfigLocal;

/// Will be the same for every federation member
#[derive(Clone, Debug, Serialize, Deserialize, Decodable, Encodable)]
pub struct DummyConfigConsensus {
    /// Will be the same for all peers
    pub tx_fee: Amount,
}

/// Will be encrypted and not shared such as private key material
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DummyConfigPrivate;

// Wire together the configs for this module
plugin_types_trait_impl_config!(
    DummyCommonInit,
    DummyGenParams,
    DummyGenParamsLocal,
    DummyGenParamsConsensus,
    DummyConfig,
    DummyConfigLocal,
    DummyConfigPrivate,
    DummyConfigConsensus,
    DummyClientConfig
);
