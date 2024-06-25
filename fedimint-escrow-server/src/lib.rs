mod db;
mod states;

use std::collections::BTreeMap;

use anyhow::bail;
use async_trait::async_trait;
use chrono::prelude::*;
use db::{DbKeyPrefix, EscrowKey, EscrowValue};
use fedimint_core::config::{
    ConfigGenModuleParams, DkgResult, ServerModuleConfig, ServerModuleConsensusConfig,
    TypedServerModuleConfig, TypedServerModuleConsensusConfig,
};
use fedimint_core::core::{DynClientConfig, IntoDynInstance, ModuleInstanceId};
use fedimint_core::db::{
    DatabaseTransaction, DatabaseVersion, IDatabaseTransactionOpsCoreTyped, NonCommittable,
    ServerMigrationFn,
};
use fedimint_core::module::audit::Audit;
use fedimint_core::module::{
    api_endpoint, ApiEndpoint, ApiError, ApiVersion, CoreConsensusVersion, InputMeta,
    ModuleConsensusVersion, ModuleInit, PeerHandle, ServerModuleInit, ServerModuleInitArgs,
    SupportedModuleApiVersions, TransactionItemAmount,
};
use fedimint_core::server::DynServerModule;
use fedimint_core::{push_db_pair_items, Amount, OutPoint, PeerId, ServerModule};
use fedimint_escrow_common::config::{
    EscrowClientConfig, EscrowConfig, EscrowConfigConsensus, EscrowConfigLocal,
    EscrowConfigPrivate, EscrowGenParams,
};
use fedimint_escrow_common::endpoints::{GET_MODULE_INFO, GET_SECRET_CODE_HASH};
use fedimint_escrow_common::{
    hash256, EscrowAction, EscrowCommonInit, EscrowConsensusItem, EscrowInput, EscrowInputError,
    EscrowModuleTypes, EscrowOutput, EscrowOutputError, CONSENSUS_VERSION,
};
use fedimint_server::config::CORE_CONSENSUS_VERSION;
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use states::{EscrowError, EscrowStates};

/// Generates the module
#[derive(Debug, Clone)]
pub struct EscrowInit;

/// ModuleInfo is the response to the GET_MODULE_INFO request
#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub buyer_pubkey: PublicKey,
    pub seller_pubkey: PublicKey,
    pub arbiter_pubkey: PublicKey,
    pub amount: Amount,
    pub secret_code_hash: [u8; 32],
    pub state: EscrowStates,
}

// // TODO: Boilerplate-code
// #[async_trait]
// impl ModuleInit for EscrowInit {
//     type Common = EscrowCommonInit;
//     const DATABASE_VERSION: DatabaseVersion = DatabaseVersion(1);

//     /// Dumps all database items for debugging
//     async fn dump_database(
//         &self,
//         dbtx: &mut DatabaseTransaction<'_>,
//         prefix_names: Vec<String>,
//     ) -> Box<dyn Iterator<Item = (String, Box<dyn erased_serde::Serialize +
// Send>)> + '_> {         // TODO: Boilerplate-code
//         let mut items: BTreeMap<String, Box<dyn erased_serde::Serialize +
// Send>> = BTreeMap::new();         let filtered_prefixes =
// DbKeyPrefix::iter().filter(|f| {             prefix_names.is_empty() ||
// prefix_names.contains(&f.to_string().to_lowercase())         });

//         for table in filtered_prefixes {
//             match table {
//                 DbKeyPrefix::Escrow => {
//                     push_db_pair_items!(
//                         dbtx, Escrow, EscrowKey,
//                         Amount,
//                         items, "Escrow"
//                     );
//                 }
//             }
//         }

//         Box::new(items.into_iter())
//     }
// }

/// Implementation of server module non-consensus functions
#[async_trait]
impl ServerModuleInit for EscrowInit {
    type Params = EscrowGenParams;

    /// Returns the version of this module
    fn versions(&self, _core: CoreConsensusVersion) -> &[ModuleConsensusVersion] {
        &[CONSENSUS_VERSION]
    }

    fn supported_api_versions(&self) -> SupportedModuleApiVersions {
        SupportedModuleApiVersions::from_raw(
            (CORE_CONSENSUS_VERSION.major, CORE_CONSENSUS_VERSION.minor),
            (CONSENSUS_VERSION.major, CONSENSUS_VERSION.minor),
            &[(0, 0)],
        )
    }

    /// Initialize the module
    async fn init(&self, args: &ServerModuleInitArgs<Self>) -> anyhow::Result<DynServerModule> {
        Ok(Escrow::new(args.cfg().to_typed()?).into())
    }

    /// Generates configs for all peers in a trusted manner for testing
    fn trusted_dealer_gen(
        &self,
        peers: &[PeerId],
        params: &ConfigGenModuleParams,
    ) -> BTreeMap<PeerId, ServerModuleConfig> {
        let params = self.parse_params(params).unwrap();
        // Generate a config for each peer
        peers
            .iter()
            .map(|&peer| {
                let config = EscrowConfig {
                    local: EscrowConfigLocal {},
                    private: EscrowConfigPrivate,
                    consensus: EscrowConfigConsensus {
                        deposit_fee: params.consensus.deposit_fee,
                    },
                };
                (peer, config.to_erased())
            })
            .collect()
    }

    /// Generates configs for all peers in an untrusted manner
    async fn distributed_gen(
        &self,
        _peers: &PeerHandle,
        params: &ConfigGenModuleParams,
    ) -> DkgResult<ServerModuleConfig> {
        let params = self.parse_params(params).unwrap();

        Ok(EscrowConfig {
            local: EscrowConfigLocal {},
            private: EscrowConfigPrivate,
            consensus: EscrowConfigConsensus {
                deposit_fee: params.consensus.deposit_fee,
            },
        }
        .to_erased())
    }

    /// Converts the consensus config into the client config
    fn get_client_config(
        &self,
        config: &ServerModuleConsensusConfig,
    ) -> anyhow::Result<EscrowClientConfig> {
        let config = EscrowConfigConsensus::from_erased(config)?;
        Ok(EscrowClientConfig {
            deposit_fee: config.deposit_fee,
        })
    }

    fn validate_config(
        &self,
        _identity: &PeerId,
        _config: ServerModuleConfig,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl IntoDynInstance for EscrowClientConfig {
    type DynType = DynClientConfig;

    fn into_dyn(self, instance_id: ModuleInstanceId) -> Self::DynType {
        DynClientConfig::new(instance_id, self)
    }
}

/// The escrow module
#[derive(Debug)]
pub struct Escrow {
    pub cfg: EscrowConfig,
}

/// Implementation of consensus for the server module
#[async_trait]
impl ServerModule for Escrow {
    /// Define the consensus types
    type Common = EscrowModuleTypes;
    type Init = EscrowInit;

    async fn consensus_proposal(
        &self,
        _dbtx: &mut DatabaseTransaction<'_>,
    ) -> Vec<EscrowConsensusItem> {
        Vec::new()
    }

    async fn process_consensus_item<'a, 'b>(
        &'a self,
        _dbtx: &mut DatabaseTransaction<'b>,
        _consensus_item: EscrowConsensusItem,
        _peer_id: PeerId,
    ) -> anyhow::Result<()> {
        bail!("The escrow module does not use consensus items");
    }

    async fn process_input<'a, 'b, 'c>(
        &'a self,
        dbtx: &mut DatabaseTransaction<'c>,
        input: &'b EscrowInput,
    ) -> Result<InputMeta, EscrowInputError> {
        let escrow_key = EscrowKey {
            escrow_id: input.escrow_id.to_string(),
        };
        let mut escrow_value: EscrowValue = dbtx
            .get_value(&escrow_key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Escrow not found"))?; // use EscrowError for this

        match input.action {
            EscrowAction::Claim => match escrow_value.state {
                EscrowStates::Open => {
                    escrow_value.state = EscrowStates::ResolvedWithoutDispute;
                }
                EscrowStates::Disputed => {
                    escrow_value.state = EscrowStates::ResolvedWithDispute;
                }
                _ => return Err(EscrowError::InvalidStateForClaimingEscrow),
            },
            EscrowAction::Dispute => match escrow_value.state {
                EscrowStates::Open => {
                    escrow_value.state = EscrowStates::Disputed;
                }
                EscrowStates::Disputed => match input.arbiter_state.as_ref() {
                    Some("buyer") => escrow_value.state = EscrowStates::WaitingforBuyer,
                    Some("seller") => escrow_value.state = EscrowStates::WaitingforSeller,
                    _ => return Err(EscrowError::InvalidArbiterState),
                },
                _ => return Err(EscrowError::InvalidStateForInitiatingDispute),
            },
            EscrowAction::Retreat => {
                escrow_value.state = EscrowStates::ResolvedWithoutDispute;
            }
        }

        dbtx.insert_entry(&escrow_key, &escrow_value).await?;
        dbtx.commit().await?;

        Ok(InputMeta {
            amount: TransactionItemAmount {
                amount: input.amount,
                fee: Amount::ZERO,
            },
            pub_key: self.key().public_key(), /* sellers public key! the one who is getting the
                                               * money! */
        })
    }

    async fn process_output<'a, 'b>(
        &'a self,
        dbtx: &mut DatabaseTransaction<'b>,
        output: &'a EscrowOutput,
        out_point: OutPoint,
    ) -> Result<TransactionItemAmount, EscrowOutputError> {
        let escrow_key = EscrowKey {
            escrow_id: output.escrow_id.to_string(),
        };
        let secret_code_hash = hash256(hash256(
            format!("{}{}{}", output.seller, output.arbiter, output.amount)
                .chars()
                .rev()
                .collect::<String>(),
        ));
        let escrow_value = EscrowValue {
            buyer_pubkey: output.buyer_pubkey,
            seller_pubkey: output.seller_pubkey,
            arbiter_pubkey: output.arbiter_pubkey,
            amount: output.amount.to_string(),
            code_hash: secret_code_hash,
            state: EscrowStates::Open,
            created_at: chrono::Utc::now().timestamp() as u64, /* set the timestamp for escrow
                                                                * creation */
            retreat_duration: output.retreat_duration,
        };

        // guardian db entry
        dbtx.insert_new_entry(
            &EscrowKey {
                escrow_id: output.escrow_id.to_string(),
            },
            &escrow_value,
        )
        .await;

        Ok(TransactionItemAmount {
            amount: output.amount,
            fee: self.cfg.consensus.deposit_fee,
        })
        // implemented! TODO : signature using public keys of buyer, seller and
        // arbiter to secure it!
    }

    async fn output_status(
        &self,
        dbtx: &mut DatabaseTransaction<'_>,
        out_point: OutPoint,
    ) -> Option<EscrowOutputOutcome> {
        unimplemented!()
    }

    async fn audit(
        &self,
        dbtx: &mut DatabaseTransaction<'_>,
        audit: &mut Audit,
        module_instance_id: ModuleInstanceId,
    ) {
        unimplemented!()
        // audit
        //     .add_items(
        //         dbtx,
        //         module_instance_id,
        //         &EscrowKey,
        //         |k, v| match k {
        //             // the fed's test account is considered an asset
        // (positive)             // should be the bitcoin we own in a
        // real module             EscrowKey(key)
        //                 if key == fed_public_key() || key ==
        // broken_fed_public_key() =>             {
        //                 v.msats as i64
        //             }
        //             // a user's funds are a federation's liability (negative)
        //             EscrowKey(_) => -(v.msats as i64),
        //         },
        //     )
        //     .await;
    }

    // api will be called in client
    fn api_endpoints(&self) -> Vec<ApiEndpoint<Self>> {
        vec![
            api_endpoint! {
                GET_MODULE_INFO,
                ApiVersion::new(0, 0),
                async |module: &Escrow, context, escrow_id: String| -> ModuleInfo {
                    module.handle_get_module_info(&mut context.dbtx().into_nc(), escrow_id).await
                }
            },
            api_endpoint! {
                GET_SECRET_CODE_HASH,
                ApiVersion::new(0, 0),
                async |module: &Escrow, context, escrow_id: String| -> Result<[u8; 32], ApiError> {
                    module.handle_get_secret_code_hash(&mut context.dbtx().into_nc(), escrow_id).await
                        .map_err(|e| ApiError::InternalError(e.to_string()))
                }
            },
        ]
    }
}

impl Escrow {
    /// Create new module instance
    pub fn new(cfg: EscrowConfig) -> Escrow {
        Escrow { cfg }
    }

    async fn handle_get_module_info(
        &self,
        dbtx: &mut DatabaseTransaction<'_, NonCommittable>,
        escrow_id: String,
    ) -> Result<ModuleInfo, ApiError> {
        let value: EscrowValue = dbtx
            .get_value(&EscrowKey { escrow_id })
            .await?
            .ok_or(EscrowError::EscrowNotFound)?;
        Ok(ModuleInfo {
            buyer_pubkey: value.buyer_pubkey,
            seller_pubkey: value.seller_pubkey,
            arbiter_pubkey: value.arbiter_pubkey,
            amount: value.amount,
            secret_code_hash: value.secret_code_hash,
            state: value.state,
        })
    }

    async fn handle_get_secret_code_hash(
        &self,
        dbtx: &mut DatabaseTransaction<'_, NonCommittable>,
        escrow_id: String,
    ) -> Result<[u8; 32], EscrowError> {
        let escrow_value: EscrowValue = dbtx
            .get_value(&EscrowKey { escrow_id })
            .await?
            .ok_or(EscrowError::EscrowNotFound)?;

        Ok(escrow_value.secret_code_hash)
    }
}
