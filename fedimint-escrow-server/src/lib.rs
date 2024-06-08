use std::collections::BTreeMap;

use anyhow::bail;
use async_trait::async_trait;
use fedimint_core::config::{
    ConfigGenModuleParams, DkgResult, ServerModuleConfig, ServerModuleConsensusConfig,
    TypedServerModuleConfig, TypedServerModuleConsensusConfig,
};
use fedimint_core::core::ModuleInstanceId;
use fedimint_core::db::{
    DatabaseTransaction, DatabaseVersion, IDatabaseTransactionOpsCoreTyped, NonCommittable,
    ServerMigrationFn,
};
use fedimint_core::module::audit::Audit;
use fedimint_core::module::{
    api_endpoint, ApiEndpoint, CoreConsensusVersion, InputMeta, ModuleConsensusVersion, ModuleInit,
    PeerHandle, ServerModuleInit, ServerModuleInitArgs, SupportedModuleApiVersions,
    TransactionItemAmount,
};
use fedimint_core::server::DynServerModule;
use fedimint_core::{push_db_pair_items, Amount, OutPoint, PeerId, ServerModule};
use fedimint_escrow_client::CODE;
use fedimint_escrow_common::config::{
    EscrowClientConfig, EscrowConfig, EscrowConfigConsensus, EscrowConfigLocal,
    EscrowConfigPrivate, EscrowGenParams,
};
use fedimint_escrow_common::{
    broken_fed_public_key, fed_public_key, hash256, ApiError, EscrowCommonInit,
    EscrowConsensusItem, EscrowInput, EscrowInputError, EscrowModuleTypes, EscrowOutput,
    EscrowOutputError, GetModuleInfoRequest, ModuleInfo, CONSENSUS_VERSION,
};
use fedimint_server::config::CORE_CONSENSUS_VERSION;
use sha2::{Digest, Sha256};

use crate::db::{DbKeyPrefix, EscrowKey, NonceKey, NonceKeyPrefix};

/// Generates the module
#[derive(Debug, Clone)]
pub struct EscrowInit;

// TODO: Boilerplate-code
#[async_trait]
impl ModuleInit for EscrowInit {
    type Common = EscrowCommonInit;
    const DATABASE_VERSION: DatabaseVersion = DatabaseVersion(1);

    /// Dumps all database items for debugging
    async fn dump_database(
        &self,
        dbtx: &mut DatabaseTransaction<'_>,
        prefix_names: Vec<String>,
    ) -> Box<dyn Iterator<Item = (String, Box<dyn erased_serde::Serialize + Send>)> + '_> {
        // TODO: Boilerplate-code
        let mut items: BTreeMap<String, Box<dyn erased_serde::Serialize + Send>> = BTreeMap::new();
        let filtered_prefixes = DbKeyPrefix::iter().filter(|f| {
            prefix_names.is_empty() || prefix_names.contains(&f.to_string().to_lowercase())
        });

        for table in filtered_prefixes {
            match table {
                DbKeyPrefix::Escrow => {
                    push_db_pair_items!(
                        dbtx, Escrow, EscrowKey,
                        Amount, // i guess it should be string (ecash)
                        items, "Escrow"
                    );
                }
            }
        }

        Box::new(items.into_iter())
    }
}

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

/// escrow module
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

    // enum of possible transitions for the state
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
                    self.delete_escrow_data(dbtx, &escrow_key.escrow_id).await?;
                }
                EscrowStates::Disputed => {
                    escrow_value.state = EscrowStates::ResolvedWithDispute;
                    self.delete_escrow_data(dbtx, &escrow_key.escrow_id).await?;
                }
                _ => return Err(anyhow!("Invalid state for claiming escrow").into()),
            },
            EscrowAction::Dispute => {
                escrow_value.state = EscrowStates::Disputed;
            }
            EscrowAction::Retreat => {
                escrow_value.state = EscrowStates::ResolvedWithoutDispute;
                self.delete_escrow_data(dbtx, &escrow_key.escrow_id).await?;
            }
        }

        dbtx.insert_entry(&escrow_key, &escrow_value).await?;
        dbtx.commit().await?;

        // todo : understand this?
        Ok(InputMeta {
            amount: TransactionItemAmount {
                amount: input.amount,
                fee: self.cfg.consensus.deposit_fee,
            },
            pub_key: self.key().public_key(), //buyers public key
        })
        // mark delete escrow_id as escrowkey after getting the funds to the
        // buyer and changing state to resolved!
    }

    async fn process_output<'a, 'b>(
        &'a self,
        dbtx: &mut DatabaseTransaction<'b>,
        output: &'a EscrowOutput,
        out_point: OutPoint,
    ) -> Result<TransactionItemAmount, EscrowOutputError> {
        let escrow_key = EscrowKey {
            uuid: output.escrow_id.to_string(),
        };
        let code_hash = hash256(CODE);
        let escrow_value = EscrowValue {
            buyer: output.buyer,
            seller: output.seller,
            arbiter: output.arbiter,
            amount: output.amount.to_string(),
            code_hash,
            state: output.state,
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
        // store info in db! and share code as escrow input
        // TODO : not happy buyer claiming the fund back will also need to be
        // implemented! TODO : signature using public keys of buyer,
        // seller and arbiter to secure it!
    }

    async fn output_status(
        &self,
        dbtx: &mut DatabaseTransaction<'_>,
        out_point: OutPoint,
    ) -> Option<EscrowOutputOutcome> {
        // check whether or not the output has been processed
        dbtx.get_value(&EscrowKey(out_point)).await
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
                async |module: &Escrow, context, request: GetModuleInfoRequest| -> ModuleInfo {
                    module.handle_get_module_info(&mut context.dbtx().into_nc(), &request).await
                }
            },
            api_endpoint! {
                GET_SECRET_CODE_HASH,
                ApiVersion::new(0, 0),
                async |module: &Escrow, context, escrow_id: String| -> Result<SecretCodeHash, EscrowError> {
                    let request = GetSecretCodeHashRequest { escrow_id };
                    module.handle_get_secret_code_hash(&mut context.dbtx().into_nc(), &request).await
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
        req: &GetModuleInfoRequest,
    ) -> Result<ModuleInfo, ApiError> {
        let escrow_value: Option<EscrowValue> = dbtx
            .get_value(&EscrowKey {
                escrow_id: req.escrow_id,
            })
            .await?;
        match escrow_value {
            Some(value) => Ok(ModuleInfo {
                buyer: value.buyer,
                seller: value.seller,
                arbiter: value.arbiter,
                amount: value.amount,
                code_hash: value.code_hash,
                state: value.state,
            }),
            None => Err(EscrowError::EscrowNotFound),
        }
    }

    async fn handle_get_secret_code_hash(
        &self,
        dbtx: &mut DatabaseTransaction<'_, NonCommittable>,
        req: &GetSecretCodeHashRequest,
    ) -> Result<[u8; 32], EscrowError> {
        let escrow_value: Option<EscrowValue> = dbtx
            .get_value(&EscrowKey {
                escrow_id: req.escrow_id.clone(),
            })
            .await?;

        match escrow_value {
            Some(value) => Ok(value.code_hash),
            None => Err(EscrowError::EscrowNotFound),
        }
    }

    async fn delete_escrow_data(
        &self,
        dbtx: &mut DatabaseTransaction<'_>,
        escrow_id: &str,
    ) -> Result<(), EscrowError> {
        let escrow_key = EscrowKey {
            escrow_id: escrow_id.to_string(),
        };

        dbtx.remove_entry(&escrow_key).await?;

        Ok(())
    }
}
