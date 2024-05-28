use std::str::FromStr as _;
use std::{ffi, iter};

use anyhow::Context;
use clap::Parser;
use fedimint_core::Amount;
use fedimint_escrow_common::config::CODE;
use secp256k1::PublicKey;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::EscrowClientModule;
use crate::api::EscrowFederationApi;

#[derive(Parser, Serialize)]
enum Command {
    CreateEscrow {
        buyer: PublicKey,
        seller: PublicKey,
        arbiter: PublicKey,
        cost: u64, //cost of ecash
    },
    EscrowInfo {
        escrow_id: Uuid,
    },
    EscrowClaim {
        escrow_id: Uuid,
        secret_code: String,
    },
    EscrowDispute {
        escrow_id: Uuid,
    },
}

pub(crate) async fn handle_cli_command(
    escrow: &EscrowClientModule,
    args: &[ffi::OsString],
) -> anyhow::Result<serde_json::Value> {
    let command =
        Command::parse_from(iter::once(&ffi::OsString::from("escrow")).chain(args.iter()));

    let res = match command {
        Command::CreateEscrow {
            buyer,
            seller,
            arbiter,
            cost,
        } => {
            // finalize_and_submit txns to burn ecash (underfunded)
            let (operation_id, out_point, escrow_id) = escrow
                .buyer_txn(Amount::from_sat(cost), buyer, seller, arbiter)
                .await?;
            // If transaction is accepted and state is opened in server, send escrow ID
            Ok(json!({
                "secret-code": CODE,
                "escrow-id": escrow_id,
                "status": "escrow opened!"
            }))
        }
        Command::EscrowInfo { escrow_id } => {
            // get escrow info corresponding to the id from db using federation api
            let request = GetModuleInfoRequest { escrow_id };
            let response: ModuleInfo = escrow
                .client_ctx
                .api()
                .request(GET_MODULE_INFO, request)
                .await?;
            Ok(serde_json::to_value(response)?)
        }
        Command::EscrowClaim {
            escrow_id,
            secret_code,
            amount,
        } => {
            // if disputed some ecash fee to arbiter also
            // otherwise normal ecash to seller
            // escrow state is closed
            // make an api call to db and get code hash, and then verify it
            let dbtx = self.db.begin_transaction().await?;
            let escrow_value: Option<EscrowValue> = dbtx
                .get_value(&EscrowKey {
                    uuid: escrow_id.to_string(),
                })
                .await?;
            match escrow_value {
                Some(value) => Ok(value.code_hash),
                None => Err(anyhow::Error::msg("Escrow not found")),
            }
            if value.code_hash != hash_secret_code(secret_code) {
                return Err(anyhow::Error::msg("Invalid secret code"));
            }
            // seller claims ecash through finalize_and_submit txn (overfunded)
            escrow.seller_txn(escrow_id, secret_code, amount).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "claimed"
            }))
        }
        Command::EscrowDispute { escrow_id } => {
            // Call the arbiter and change the state to disputed
            escrow.initiate_dispute(escrow_id).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "disputed"
            }))
        } // arbiter can tell fed to pay ecash to buyer
    };

    Ok(res)
}

fn hash_secret_code(secret_code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret_code);
    hex::encode(hasher.finalize())
}
