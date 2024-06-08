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

use super::EscrowClientModule;
use crate::api::EscrowFederationApi;

// make sure you are sending clone of things, not references or direct sending!

// TODO: we need cli-commands as well as API endpoints for these commands!
#[derive(Parser, Serialize)]
enum Command {
    CreateEscrow {
        buyer: PublicKey, // decide on this later, hexadecimal string or bytes ?
        seller: PublicKey,
        arbiter: PublicKey,
        cost: u64, // actual cost of product
    },
    EscrowInfo {
        escrow_id: String,
    },
    EscrowClaim {
        escrow_id: String,
        secret_code: String,
    },
    EscrowDispute {
        escrow_id: String,
        arbiter: PublicKey,
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
            // finalize_and_submit txns to lock ecash by underfunding
            // how to call this method?
            let (operation_id, out_point, escrow_id) = escrow
                .buyer_txn(Amount::from_sat(cost), buyer, seller, arbiter)
                .await?;
            // even though unique transaction id will be assigned, escrow id will used to
            // collectively get all data related to the escrow
            pub const CODE: String =
                hash256((vec![buyer, seller, arbiter, cost].concat()).reverse());
            // If transaction is accepted and state is opened in server, share escrow ID and
            // CODE
            Ok(json!({
                "secret-code": CODE, // shared by buyer out of band to seller
                "escrow-id": escrow_id,
                "state": "escrow opened!"
            }))
        }
        Command::EscrowInfo { escrow_id } => {
            // get escrow info corresponding to the id from db using federation api
            let response: ModuleInfo = escrow
                .client_ctx
                .api()
                .request(GET_MODULE_INFO, escrow_id)
                .await?;
            Ok(serde_json::to_value(response)?)
        }
        Command::EscrowClaim {
            escrow_id,
            secret_code,
        } => {
            // make an api call to server db and get code hash, and then verify it
            let response: [u8; 32] = escrow
                .client_ctx
                .api()
                .request(GET_SECRET_CODE_HASH, escrow_id)
                .await?;
            if response.code_hash != hash256(secret_code) {
                return Err(anyhow::Error::msg("Invalid secret code"));
            }
            // seller claims ecash through finalize_and_submit txn by overfunding
            escrow
                .seller_txn(escrow_id, secret_code, response.amount)
                .await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved"
            }))

            // first handle server side state, then client side!
        }
        Command::EscrowDispute { escrow_id, arbiter } => {
            // by buyer and seller both
            // Call the arbiter and change the state to disputed
            // the arbiter will take a fee (decided off band)
            escrow.initiate_dispute(escrow_id).await?;
            // initiate_dispute probably shouldn't be a Transaction, but a consensus item
            // submitted via guardian api endpoint.
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "disputed"
            }))
            // out of band notification to arbiter give escrow_id to get the
            // contract detail
        }
        Command::EscrowRetreat { escrow_id } => {
            // buyer can retreat the escrow if the seller doesn't act within a time period!
            // what should be that time period?
            let response: ModuleInfo = escrow
                .client_ctx
                .api()
                .request(GET_MODULE_INFO, escrow_id)
                .await?;
            escrow.retreat_txn(escrow_id, response.amount).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved!"
            }))
        }
    };

    // TODO: arbiter release funds commands,  arbiter can tell fed to pay ecash to
    // buyer

    Ok(res)
}
