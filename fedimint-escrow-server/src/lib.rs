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
use fedimint_core::{Amount, OutPoint, PeerId, ServerModule};
use fedimint_escrow_common::config::{
    EscrowClientConfig, EscrowConfig, EscrowConfigConsensus, EscrowConfigLocal,
    EscrowConfigPrivate, EscrowGenParams,
};
use fedimint_escrow_common::endpoints::{ModuleInfo, GET_MODULE_INFO};
use fedimint_escrow_common::{
    hash256, EscrowCommonInit, EscrowConsensusItem, EscrowInput, EscrowInputError,
    EscrowModuleTypes, EscrowOutput, EscrowOutputError, EscrowOutputOutcome, EscrowStates,
    CONSENSUS_VERSION,
};
use fedimint_server::config::CORE_CONSENSUS_VERSION;
use secp256k1::{Message, PublicKey, Secp256k1, Signature};
use states::EscrowError;

/// Generates the module
#[derive(Debug, Clone)]
pub struct EscrowInit;

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
                        max_arbiter_fee_bps: params.consensus.max_arbiter_fee_bps,
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
                max_arbiter_fee_bps: params.consensus.max_arbiter_fee_bps,
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
            .ok_or_else(|| EscrowError::EscrowNotFound)?;

        let claimer = escrow_value.seller_pubkey;

        match input {
            EscrowInput::ClamingWithoutDispute(escrow_input) => {
                // the secret code when hashed should be the same as the one in the db
                if escrow_value.secret_code_hash != hash256(escrow_input.secret_code) {
                    return Err(EscrowError::InvalidSecretCode);
                }
                escrow_value.state = EscrowStates::ResolvedWithoutDispute;
            }
            EscrowInput::Disputing(escrow_input) => {
                // Determine who is disputing
                let disputer = if escrow_input.disputer == escrow_value.buyer_pubkey {
                    Disputer::Buyer
                } else if escrow_input.disputer == escrow_value.seller_pubkey {
                    Disputer::Seller
                } else {
                    return Err(EscrowError::UnauthorizedToDispute);
                };

                match escrow_value.state {
                    EscrowStates::Open => {
                        escrow_value.state = match disputer {
                            Disputer::Buyer => EscrowStates::DisputedByBuyer,
                            Disputer::Seller => EscrowStates::DisputedBySeller,
                        };
                    }
                    _ => return Err(EscrowError::InvalidStateForInitiatingDispute),
                }
            }
            EscrowInput::ArbiterDecision(escrow_input) => {
                // the escrow state should be disputed for the arbiter to take decision
                let escrow_value: ModuleInfo = escrow
                    .client_ctx
                    .api()
                    .request(GET_MODULE_INFO, escrow_id)
                    .await?;
                if escrow_value.state != EscrowStates::Disputed {
                    return Err(EscrowError::EscrowNotDisputed);
                }
                // check the signature of arbiter
                let secp = Secp256k1::new();
                if !secp
                    .verify(
                        &escrow_input.message,
                        &escrow_input.signature,
                        &escrow_value.arbiter_pubkey,
                    )
                    .is_ok()
                {
                    return Err(EscrowError::InvalidArbiter);
                }

                // Validate arbiter's fee
                if escrow_input.amount > escrow_value.max_arbiter_fee {
                    return Err(anyhow::anyhow!("Arbiter fee exceeds the maximum allowed"));
                } else {
                    // the contract amount is the amount of ecash in the contract - arbiter fee
                    escrow_value.amount = escrow_value.amount - escrow_input.amount;
                }

                // Update the escrow state based on the arbiter's decision
                match escrow_input.arbiter_decision.as_ref() {
                    ArbiterDecision::BuyerWins => {
                        escrow_value.state = EscrowStates::WaitingforBuyerToClaim;
                    }
                    ArbiterDecision::SellerWins => {
                        escrow_value.state = EscrowStates::WaitingforSellerToClaim;
                    }
                }
            }
            EscrowInput::ClaimingAfterDispute(escrow_input) => match escrow_value.state {
                EscrowStates::WaitingforBuyerToClaim => {
                    escrow_value.state = EscrowStates::ResolvedWithDispute;
                    claimer = escrow_value.buyer_pubkey;
                }
                EscrowStates::WaitingforSellerToClaim => {
                    escrow_value.state = EscrowStates::ResolvedWithDispute;
                }
                _ => return Err(EscrowError::InvalidStateForClaimingEscrow),
            },
        }

        // Update the escrow value in the database
        dbtx.insert_entry(&escrow_key, &escrow_value).await?;
        dbtx.commit().await?;

        Ok(InputMeta {
            amount: TransactionItemAmount {
                amount: input.amount,
                fee: Amount::ZERO,
            },
            pub_key: claimer, // the one who is getting the ecash
        })
    }

    async fn process_output<'a, 'b>(
        &'a self,
        dbtx: &mut DatabaseTransaction<'b>,
        output: &'a EscrowOutput,
        out_point: OutPoint,
    ) -> Result<TransactionItemAmount, EscrowOutputError> {
        let escrow_key = EscrowKey {
            escrow_id: output.escrow_id,
        };
        let escrow_value = EscrowValue {
            buyer_pubkey: output.buyer_pubkey,
            seller_pubkey: output.seller_pubkey,
            arbiter_pubkey: output.arbiter_pubkey,
            amount: output.amount.to_string(),
            secret_code_hash: output.secret_code_hash,
            max_arbiter_fee: output.max_arbiter_fee,
            state: EscrowStates::Open,
            created_at: chrono::Utc::now().timestamp(), // set the timestamp for escrow creation
        };

        // guardian db entry
        dbtx.insert_new_entry(
            &EscrowKey {
                escrow_id: output.escrow_id,
            },
            &escrow_value,
        )
        .await;

        Ok(TransactionItemAmount {
            amount: output.amount,
            fee: self.cfg.consensus.deposit_fee,
        })
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
        vec![api_endpoint! {
            GET_MODULE_INFO,
            ApiVersion::new(0, 0),
            async |module: &Escrow, context, escrow_id: String| -> ModuleInfo {
                module.handle_get_module_info(&mut context.dbtx().into_nc(), escrow_id).await
            }
        }]
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
        let escrow_value: EscrowValue = dbtx
            .get_value(&EscrowKey { escrow_id })
            .await?
            .ok_or(EscrowError::EscrowNotFound)?;
        Ok(ModuleInfo {
            buyer_pubkey: escrow_value.buyer_pubkey,
            seller_pubkey: escrow_value.seller_pubkey,
            arbiter_pubkey: escrow_value.arbiter_pubkey,
            amount: escrow_value.amount,
            secret_code_hash: escrow_value.secret_code_hash,
            state: escrow_value.state,
            max_arbiter_fee: escrow_value.max_arbiter_fee,
            created_at: escrow_value.created_at,
        })
    }
}
