mod db;

use std::collections::BTreeMap;

use anyhow::bail;
use async_trait::async_trait;
pub use db::EscrowValue;
use db::{DbKeyPrefix, EscrowKey};
use fedimint_core::config::{
    ConfigGenModuleParams, DkgResult, ServerModuleConfig, ServerModuleConsensusConfig,
    TypedServerModuleConfig, TypedServerModuleConsensusConfig,
};
use fedimint_core::core::ModuleInstanceId;
use fedimint_core::db::{
    DatabaseTransaction, DatabaseVersion, IDatabaseTransactionOpsCoreTyped, NonCommittable,
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
use fedimint_escrow_common::endpoints::{EscrowInfo, GET_MODULE_INFO};
use fedimint_escrow_common::{
    hash256, ArbiterDecision, Disputer, EscrowCommonInit, EscrowConsensusItem, EscrowInput,
    EscrowInputError, EscrowModuleTypes, EscrowOutput, EscrowOutputError, EscrowOutputOutcome,
    EscrowStates, MODULE_CONSENSUS_VERSION,
};
use fedimint_server::config::CORE_CONSENSUS_VERSION;
use futures::StreamExt;
use secp256k1::{Message, Secp256k1};
use strum::IntoEnumIterator;

/// Generates the module
#[derive(Debug, Clone)]
pub struct EscrowInit;

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
        let mut items: BTreeMap<String, Box<dyn erased_serde::Serialize + Send>> = BTreeMap::new();
        let filtered_prefixes = DbKeyPrefix::iter().filter(|f| {
            prefix_names.is_empty() || prefix_names.contains(&f.to_string().to_lowercase())
        });

        for prefix in filtered_prefixes {
            match prefix {
                DbKeyPrefix::Escrow => {
                    push_db_pair_items!(
                        dbtx,
                        DbKeyPrefix::Escrow,
                        EscrowKey,
                        EscrowValue,
                        items,
                        "Escrow"
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
        &[MODULE_CONSENSUS_VERSION]
    }

    fn supported_api_versions(&self) -> SupportedModuleApiVersions {
        SupportedModuleApiVersions::from_raw(
            (CORE_CONSENSUS_VERSION.major, CORE_CONSENSUS_VERSION.minor),
            (
                MODULE_CONSENSUS_VERSION.major,
                MODULE_CONSENSUS_VERSION.minor,
            ),
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
            max_arbiter_fee_bps: config.max_arbiter_fee_bps,
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
        tracing::info!("Processing input: {:?}", input);
        match input {
            EscrowInput::ClamingWithoutDispute(escrow_input) => {
                let mut escrow_value = self
                    .get_escrow_value(dbtx, escrow_input.escrow_id.clone())
                    .await?;

                // check the signature of seller
                let secp = Secp256k1::new();
                let message = Message::from_slice(&escrow_input.hashed_message).expect("32 bytes");
                let (xonly_pubkey, _parity) = escrow_value.seller_pubkey.x_only_public_key();

                if !secp
                    .verify_schnorr(&escrow_input.signature, &message, &xonly_pubkey)
                    .is_ok()
                {
                    return Err(EscrowInputError::InvalidSeller);
                }

                // the secret code when hashed should be the same as the one in the db
                if escrow_value.secret_code_hash != hash256(escrow_input.secret_code.clone()) {
                    return Err(EscrowInputError::InvalidSecretCode);
                }
                escrow_value.state = EscrowStates::ResolvedWithoutDispute;

                // Update the escrow value in the database
                let escrow_key = self.get_escrow_key(escrow_input.escrow_id.clone()).await;
                dbtx.insert_entry(&escrow_key, &escrow_value).await;

                Ok(InputMeta {
                    amount: TransactionItemAmount {
                        amount: escrow_input.amount,
                        fee: Amount::ZERO,
                    },
                    pub_key: escrow_value.seller_pubkey, // the one who is getting the ecash
                })
            }
            EscrowInput::Disputing(escrow_input) => {
                let mut escrow_value = self
                    .get_escrow_value(dbtx, escrow_input.escrow_id.clone())
                    .await?;

                // Determine who is disputing
                let disputer = if escrow_input.disputer == escrow_value.buyer_pubkey {
                    Disputer::Buyer
                } else if escrow_input.disputer == escrow_value.seller_pubkey {
                    Disputer::Seller
                } else {
                    return Err(EscrowInputError::UnauthorizedToDispute);
                };

                // check the signature of disputer
                let secp = Secp256k1::new();
                let message = Message::from_slice(&escrow_input.hashed_message).expect("32 bytes");
                let xonly_pubkey = match disputer {
                    Disputer::Buyer => {
                        let (xonly, _parity) = escrow_value.buyer_pubkey.x_only_public_key();
                        xonly
                    }
                    Disputer::Seller => {
                        let (xonly, _parity) = escrow_value.seller_pubkey.x_only_public_key();
                        xonly
                    }
                };

                if !secp
                    .verify_schnorr(&escrow_input.signature, &message, &xonly_pubkey)
                    .is_ok()
                {
                    return Err(EscrowInputError::InvalidArbiter);
                }

                match escrow_value.state {
                    EscrowStates::Open => {
                        escrow_value.state = match disputer {
                            Disputer::Buyer => EscrowStates::DisputedByBuyer,
                            Disputer::Seller => EscrowStates::DisputedBySeller,
                        };
                    }
                    _ => return Err(EscrowInputError::InvalidStateForInitiatingDispute),
                }

                // Update the escrow value in the database
                let escrow_key = self.get_escrow_key(escrow_input.escrow_id.clone()).await;
                dbtx.insert_entry(&escrow_key, &escrow_value).await;

                Ok(InputMeta {
                    amount: TransactionItemAmount {
                        amount: Amount::ZERO,
                        fee: Amount::ZERO,
                    },
                    pub_key: escrow_input.disputer,
                })
            }
            EscrowInput::ArbiterDecision(escrow_input) => {
                let mut escrow_value = self
                    .get_escrow_value(dbtx, escrow_input.escrow_id.clone())
                    .await?;

                // the escrow state should be disputed for the arbiter to take decision
                if escrow_value.state != EscrowStates::DisputedByBuyer
                    && escrow_value.state != EscrowStates::DisputedBySeller
                {
                    return Err(EscrowInputError::EscrowNotDisputed);
                }

                // check the signature of arbiter
                let secp = Secp256k1::new();
                let message = Message::from_slice(&escrow_input.hashed_message).expect("32 bytes");
                let (xonly_pubkey, _parity) = escrow_value.arbiter_pubkey.x_only_public_key();

                if !secp
                    .verify_schnorr(&escrow_input.signature, &message, &xonly_pubkey)
                    .is_ok()
                {
                    return Err(EscrowInputError::InvalidArbiter);
                }

                // Validate arbiter's fee
                if escrow_input.amount > escrow_value.max_arbiter_fee {
                    return Err(EscrowInputError::ArbiterFeeExceedsMaximum);
                } else {
                    // the contract amount is the amount of ecash in the contract - arbiter fee
                    escrow_value.amount = escrow_value.amount - escrow_input.amount;
                }

                // Update the escrow state based on the arbiter's decision
                match escrow_input.arbiter_decision {
                    ArbiterDecision::BuyerWins => {
                        escrow_value.state = EscrowStates::WaitingforBuyerToClaim;
                    }
                    ArbiterDecision::SellerWins => {
                        escrow_value.state = EscrowStates::WaitingforSellerToClaim;
                    }
                }

                // Update the escrow value in the database
                let escrow_key = self.get_escrow_key(escrow_input.escrow_id.clone()).await;
                dbtx.insert_entry(&escrow_key, &escrow_value).await;

                Ok(InputMeta {
                    amount: TransactionItemAmount {
                        amount: escrow_input.amount,
                        fee: Amount::ZERO,
                    },
                    pub_key: escrow_value.arbiter_pubkey, // the one who is getting the ecash
                })
            }
            EscrowInput::ClaimingAfterDispute(escrow_input) => {
                let mut escrow_value = self
                    .get_escrow_value(dbtx, escrow_input.escrow_id.clone())
                    .await?;
                match escrow_value.state {
                    EscrowStates::WaitingforBuyerToClaim => {
                        // check the signature of buyer
                        let secp = Secp256k1::new();
                        let message =
                            Message::from_slice(&escrow_input.hashed_message).expect("32 bytes");
                        let (xonly_pubkey, _parity) = escrow_value.buyer_pubkey.x_only_public_key();

                        if !secp
                            .verify_schnorr(&escrow_input.signature, &message, &xonly_pubkey)
                            .is_ok()
                        {
                            return Err(EscrowInputError::InvalidArbiter);
                        }
                        escrow_value.state = EscrowStates::ResolvedWithDispute;

                        // Update the escrow value in the database
                        let escrow_key = self.get_escrow_key(escrow_input.escrow_id.clone()).await;
                        dbtx.insert_entry(&escrow_key, &escrow_value).await;

                        Ok(InputMeta {
                            amount: TransactionItemAmount {
                                amount: escrow_input.amount,
                                fee: Amount::ZERO,
                            },
                            pub_key: escrow_value.seller_pubkey, // the one who is getting the ecash
                        })
                    }
                    EscrowStates::WaitingforSellerToClaim => {
                        // check the signature of buyer
                        let secp = Secp256k1::new();
                        let message =
                            Message::from_slice(&escrow_input.hashed_message).expect("32 bytes");
                        let (xonly_pubkey, _parity) =
                            escrow_value.seller_pubkey.x_only_public_key();

                        if !secp
                            .verify_schnorr(&escrow_input.signature, &message, &xonly_pubkey)
                            .is_ok()
                        {
                            return Err(EscrowInputError::InvalidArbiter);
                        }
                        escrow_value.state = EscrowStates::ResolvedWithDispute;

                        // Update the escrow value in the database
                        let escrow_key = self.get_escrow_key(escrow_input.escrow_id.clone()).await;
                        dbtx.insert_entry(&escrow_key, &escrow_value).await;

                        Ok(InputMeta {
                            amount: TransactionItemAmount {
                                amount: escrow_input.amount,
                                fee: Amount::ZERO,
                            },
                            pub_key: escrow_value.seller_pubkey, // the one who is getting the ecash
                        })
                    }
                    _ => return Err(EscrowInputError::InvalidStateForClaimingEscrow),
                }
            }
        }
    }

    async fn process_output<'a, 'b>(
        &'a self,
        dbtx: &mut DatabaseTransaction<'b>,
        output: &'a EscrowOutput,
        _out_point: OutPoint,
    ) -> Result<TransactionItemAmount, EscrowOutputError> {
        if self
            .get_escrow_value(dbtx, output.escrow_id.clone())
            .await
            .is_ok()
        {
            return Err(EscrowOutputError::EscrowAlreadyExists);
        }
        let escrow_key = EscrowKey {
            escrow_id: output.escrow_id.clone(),
        };
        let escrow_value = EscrowValue {
            buyer_pubkey: output.buyer_pubkey,
            seller_pubkey: output.seller_pubkey,
            arbiter_pubkey: output.arbiter_pubkey,
            amount: output.amount,
            secret_code_hash: output.secret_code_hash.clone(),
            max_arbiter_fee: output.max_arbiter_fee,
            state: EscrowStates::Open,
        };

        // guardian db entry
        dbtx.insert_new_entry(&escrow_key, &escrow_value).await;

        Ok(TransactionItemAmount {
            amount: output.amount,
            fee: self.cfg.consensus.deposit_fee,
        })
    }

    async fn output_status(
        &self,
        _dbtx: &mut DatabaseTransaction<'_>,
        _out_point: OutPoint,
    ) -> Option<EscrowOutputOutcome> {
        Some(EscrowOutputOutcome {})
    }

    async fn audit(
        &self,
        _dbtx: &mut DatabaseTransaction<'_>,
        _audit: &mut Audit,
        _module_instance_id: ModuleInstanceId,
    ) {
        // unimplemented!()
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

    // api will be called in client by GET_MODULE_INFO endpoint
    fn api_endpoints(&self) -> Vec<ApiEndpoint<Self>> {
        vec![api_endpoint! {
            GET_MODULE_INFO,
            ApiVersion::new(0, 0),
            async |module: &Escrow, context, escrow_id: String| -> EscrowInfo {
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
    ) -> Result<EscrowInfo, ApiError> {
        let escrow_value: EscrowValue = dbtx
            .get_value(&EscrowKey { escrow_id })
            .await
            .ok_or_else(|| ApiError::not_found("Escrow not found".to_owned()))?;
        let escrow_info = EscrowInfo {
            buyer_pubkey: escrow_value.buyer_pubkey,
            seller_pubkey: escrow_value.seller_pubkey,
            arbiter_pubkey: escrow_value.arbiter_pubkey,
            amount: escrow_value.amount,
            secret_code_hash: escrow_value.secret_code_hash,
            state: escrow_value.state,
            max_arbiter_fee: escrow_value.max_arbiter_fee,
        };
        Ok(escrow_info)
    }

    // get the escrow value from the database using the escrow id
    async fn get_escrow_value<'a>(
        &self,
        dbtx: &mut DatabaseTransaction<'a>,
        escrow_id: String,
    ) -> Result<EscrowValue, EscrowInputError> {
        let escrow_key = self.get_escrow_key(escrow_id).await;
        dbtx.get_value(&escrow_key)
            .await
            .ok_or_else(|| EscrowInputError::EscrowNotFound)
    }

    // get the escrow key from the escrow id
    async fn get_escrow_key<'a>(&self, escrow_id: String) -> EscrowKey {
        EscrowKey { escrow_id }
    }
}
