use std::str::FromStr as _;
use std::{ffi, iter};

use anyhow::Context;
use chrono::prelude::*;
use clap::Parser;
use fedimint_core::Amount;
use fedimint_escrow_common::endpoints::ModuleInfo;
use fedimint_escrow_common::EscrowClientModule;
use secp256k1::PublicKey;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::EscrowStates;
use crate::api::EscrowFederationApi;

// TODO: we need cli-commands as well as API endpoints for these commands!
#[derive(Parser, Serialize)]
enum Command {
    escrow {
        seller: PublicKey, // decide on this later, hexadecimal string or bytes ?
        arbiter: PublicKey,
        cost: Amount,          // actual cost of product
        retreat_duration: u64, // in seconds
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
        arbiter_fee: Amount,
    },
}

/// Handles the CLI command for the escrow module
pub(crate) async fn handle_cli_command(
    escrow: &EscrowClientModule,
    args: &[ffi::OsString],
) -> anyhow::Result<serde_json::Value> {
    let command =
        Command::parse_from(iter::once(&ffi::OsString::from("escrow")).chain(args.iter()));

    let res = match command {
        Command::Escrow {
            seller_pubkey,
            arbiter_pubkey,
            cost,
            retreat_duration,
        } => {
            // Create escrow id by hashing seller, arbiter, amount
            let escrow_id = hash256(format!("{}{}{}", seller_pubkey, arbiter_pubkey, cost));

            // finalize_and_submit txns to lock ecash by underfunding
            let (operation_id, out_point) = escrow
                .create_escrow(
                    Amount::from_sat(cost),
                    seller_pubkey,
                    arbiter_pubkey,
                    retreat_duration,
                    escrow_id.clone(),
                )
                .await?;

            // Generate the secret code by hashing seller, arbiter and cost in reverse order
            let code = hash256(
                format!("{}{}{}", seller_pubkey, arbiter_pubkey, cost)
                    .chars()
                    .rev()
                    .collect::<String>(),
            );

            // If transaction is accepted and state is opened in server, share escrow ID and
            // CODE
            Ok(json!({
                "secret-code": code, // shared by buyer out of band to seller
                "escrow-id": escrow_id, // even though unique transaction id will be assigned, escrow id will used to collectively get all data related to the escrow
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
            Ok(json!({
                "buyer_pubkey": response.buyer_pubkey,
                "seller_pubkey": response.seller_pubkey,
                "arbiter_pubkey": response.arbiter_pubkey,
                "amount": response.amount,
                "state": response.state,
                // code_hash is intentionally omitted to not expose it in the response
            }))
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
            if response.state == EscrowStates::Disputed {
                return Err(EscrowError::EscrowDisputed);
            }
            if response.state != EscrowState::WaitingforSeller || EscrowState::Open {
                return Err(EscrowError::ArbiterNotDecided);
            }
            if response.code_hash != hash256(secret_code) {
                return Err(EscrowError::InvalidSecretCode);
            }
            // seller claims ecash through finalize_and_submit txn by overfunding
            escrow
                .claim_escrow(escrow_id, secret_code, response.amount)
                .await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved"
            }))

            // first handle server side state, then client side!
        }
        Command::EscrowDispute {
            escrow_id,
            arbiter_fee,
        } => {
            // the arbiter will take a fee (decided off band)
            escrow.initiate_dispute(escrow_id, arbiter_fee).await?;

            Ok(json!({
                "escrow_id": escrow_id,
                "status": "disputed"
            }))
            // out of band notification to arbiter give escrow_id to get the
            // contract detail
        }
        Command::EscrowRetreat { escrow_id } => {
            // buyer can retreat the escrow if the seller doesn't act within a time period!
            // also when the arbiter decides the ecash should be given to buyer, this
            // command would be used!
            let response: ModuleInfo = escrow
                .client_ctx
                .api()
                .request(GET_MODULE_INFO, escrow_id)
                .await?;
            if response.state == EscrowStates::Disputed {
                return Err(EscrowError::EscrowDisputed);
            }
            if response.state != EscrowState::WaitingforBuyer || EscrowState::Open {
                return Err(EscrowError::ArbiterNotDecided);
            }
            // the state should be waiting for buyer to claim the ecash as arbiter has
            // decided
            let current_timestamp = chrono::Utc::now().timestamp() as u64;
            let retreat_duration = response.retreat_duration; // time duration is set by the buyer while creating the escrow
            if current_timestamp - response.created_at < retreat_duration {
                return Err(EscrowError::RetreatTimeNotPassed);
            }
            escrow.escrow_retreat(escrow_id, response.amount).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved!"
            }))
        }
        Command::EscrowArbiterDecision {
            escrow_id,
            decision,
        } => {
            // arbiter will decide the ecash should be given to buyer or seller and change
            // the state of escrow!
            let response: ModuleInfo = escrow
                .client_ctx
                .api()
                .request(GET_MODULE_INFO, escrow_id)
                .await?;
            if response.state != EscrowStates::Disputed {
                return Err(EscrowError::EscrowNotDisputed);
            }
            if response.arbiter != self.key().public_key() {
                return Err(EscrowError::ArbiterNotMatched);
            }
            // the arbiter can act only after the time decided by the buyer has passed
            let current_timestamp = chrono::Utc::now().timestamp() as u64;
            let retreat_duration = response.retreat_duration; // time duration is set by the buyer while creating the escrow
            if current_timestamp - response.created_at < retreat_duration {
                return Err(EscrowError::RetreatTimeNotPassed);
            }
            // the arbiter will take a fee (decided off band)
            // decision has 2 values, buyer or seller.
            escrow.arbiter_txn(escrow_id, decision).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved!"
            }))
        }
    };

    Ok(res)
}
