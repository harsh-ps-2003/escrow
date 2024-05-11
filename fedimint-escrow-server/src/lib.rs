use std::collections::BTreeMap;

use anyhow::bail;
use async_trait::async_trait;
use fedimint_core::config::{
    ConfigGenModuleParams, DkgResult, ServerModuleConfig, ServerModuleConsensusConfig,
    TypedServerModuleConfig, TypedServerModuleConsensusConfig,
};
use fedimint_core::core::ModuleInstanceId;
use fedimint_core::db::{
    DatabaseTransaction, DatabaseVersion, IDatabaseTransactionOpsCoreTyped, ServerMigrationFn,
};
use fedimint_core::module::audit::Audit;
use fedimint_core::module::{
    ApiEndpoint, CoreConsensusVersion, InputMeta, ModuleConsensusVersion, ModuleInit, PeerHandle,
    ServerModuleInit, ServerModuleInitArgs, SupportedModuleApiVersions, TransactionItemAmount,
};
use fedimint_core::server::DynServerModule;
use fedimint_core::{push_db_pair_items, Amount, OutPoint, PeerId, ServerModule};
use fedimint_escrow_common::config::{
    EscrowClientConfig, EscrowConfig, EscrowConfigConsensus, EscrowConfigLocal,
    EscrowConfigPrivate, EscrowGenParams,
};
use fedimint_escrow_common::{
    broken_fed_public_key, fed_public_key, EscrowCommonInit, EscrowConsensusItem, EscrowInput,
    EscrowInputError, EscrowModuleTypes, EscrowOutput, EscrowOutputError, EscrowOutputOutcome,
    CONSENSUS_VERSION,
};
use fedimint_server::config::CORE_CONSENSUS_VERSION;
use futures::{FutureExt, StreamExt};
use strum::IntoEnumIterator;

use crate::db::{
    migrate_to_v1, DbKeyPrefix, EscrowFundsKeyV1, EscrowFundsPrefixV1, EscrowOutcomeKey,
    EscrowOutcomePrefix,
};

pub mod db;
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
                DbKeyPrefix::Funds => {
                    push_db_pair_items!(
                        dbtx,
                        EscrowFundsPrefixV1,
                        EscrowFundsKeyV1,
                        Amount,
                        items,
                        "Escrow Funds"
                    );
                }
                DbKeyPrefix::Outcome => {
                    push_db_pair_items!(
                        dbtx,
                        EscrowOutcomePrefix,
                        EscrowOutcomeKey,
                        EscrowOutputOutcome,
                        items,
                        "Escrow Outputs"
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

    /// DB migrations to move from old to newer versions
    fn get_database_migrations(&self) -> BTreeMap<DatabaseVersion, ServerMigrationFn> {
        let mut migrations: BTreeMap<DatabaseVersion, ServerMigrationFn> = BTreeMap::new();
        migrations.insert(DatabaseVersion(0), move |dbtx| migrate_to_v1(dbtx).boxed());
        migrations
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
                        tx_fee: params.consensus.tx_fee,
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
                tx_fee: params.consensus.tx_fee,
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
            tx_fee: config.tx_fee,
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
        let current_funds = dbtx
            .get_value(&EscrowFundsKeyV1(input.account))
            .await
            .unwrap_or(Amount::ZERO);

        // verify user has enough funds or is using the fed account
        if input.amount > current_funds
            && fed_public_key() != input.account
            && broken_fed_public_key() != input.account
        {
            return Err(EscrowInputError::NotEnoughFunds);
        }

        // Subtract funds from normal user, or print funds for the fed
        let updated_funds = if fed_public_key() == input.account {
            current_funds + input.amount
        } else if broken_fed_public_key() == input.account {
            // The printer is broken
            current_funds
        } else {
            current_funds - input.amount
        };

        dbtx.insert_entry(&EscrowFundsKeyV1(input.account), &updated_funds)
            .await;

        Ok(InputMeta {
            amount: TransactionItemAmount {
                amount: input.amount,
                fee: self.cfg.consensus.tx_fee,
            },
            // IMPORTANT: include the pubkey to validate the user signed this tx
            pub_key: input.account,
        })
    }

    async fn process_output<'a, 'b>(
        &'a self,
        dbtx: &mut DatabaseTransaction<'b>,
        output: &'a EscrowOutput,
        out_point: OutPoint,
    ) -> Result<TransactionItemAmount, EscrowOutputError> {
        // Add output funds to the user's account
        let current_funds = dbtx.get_value(&EscrowFundsKeyV1(output.account)).await;
        let updated_funds = current_funds.unwrap_or(Amount::ZERO) + output.amount;
        dbtx.insert_entry(&EscrowFundsKeyV1(output.account), &updated_funds)
            .await;

        // Update the output outcome the user can query
        let outcome = EscrowOutputOutcome(updated_funds, output.account);
        dbtx.insert_entry(&EscrowOutcomeKey(out_point), &outcome)
            .await;

        Ok(TransactionItemAmount {
            amount: output.amount,
            fee: self.cfg.consensus.tx_fee,
        })
    }

    async fn output_status(
        &self,
        dbtx: &mut DatabaseTransaction<'_>,
        out_point: OutPoint,
    ) -> Option<EscrowOutputOutcome> {
        // check whether or not the output has been processed
        dbtx.get_value(&EscrowOutcomeKey(out_point)).await
    }

    async fn audit(
        &self,
        dbtx: &mut DatabaseTransaction<'_>,
        audit: &mut Audit,
        module_instance_id: ModuleInstanceId,
    ) {
        audit
            .add_items(
                dbtx,
                module_instance_id,
                &EscrowFundsPrefixV1,
                |k, v| match k {
                    // the fed's test account is considered an asset (positive)
                    // should be the bitcoin we own in a real module
                    EscrowFundsKeyV1(key)
                        if key == fed_public_key() || key == broken_fed_public_key() =>
                    {
                        v.msats as i64
                    }
                    // a user's funds are a federation's liability (negative)
                    EscrowFundsKeyV1(_) => -(v.msats as i64),
                },
            )
            .await;
    }

    fn api_endpoints(&self) -> Vec<ApiEndpoint<Self>> {
        Vec::new()
    }
}

impl Escrow {
    /// Create new module instance
    pub fn new(cfg: EscrowConfig) -> Escrow {
        Escrow { cfg }
    }
}
