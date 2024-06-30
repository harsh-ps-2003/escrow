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
    Escrow {
        seller_pubkey: PublicKey, // decide on this later, hexadecimal string or bytes ?
        arbiter_pubkey: PublicKey,
        cost: Amount,             // actual cost of product
        max_arbiter_fee_bps: u16, // maximum arbiter fee in basis points
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
    },
    EscrowArbiterDecision {
        escrow_id: String,
        decision: ArbiterDecision,
        arbiter_fee_bps: u16, // arbiter fee in basis points out of predecided maximum arbiters fee
    },
    BuyerClaim {
        escrow_id: String,
    },
    SellerClaim {
        escrow_id: String,
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
            max_arbiter_fee_bps,
        } => {
            // Create a random escrow id, which will only be known by the buyer, and will be
            // shared to seller or arbiter by the buyer
            let escrow_id: [u8; 32] = rand::random();

            // Generate a random secret code
            let secret_code: [u8; 32] = rand::random();
            let secret_code_hash = hash256(&secret_code);

            // finalize_and_submit txns to lock ecash by underfunding
            let (operation_id, out_point) = escrow
                .create_escrow(
                    Amount::from_sat(cost),
                    seller_pubkey,
                    arbiter_pubkey,
                    escrow_id.clone(),
                    secret_code_hash,
                    max_arbiter_fee_bps,
                )
                .await?;

            // If transaction is accepted and state is opened in server, share escrow ID and
            // CODE
            Ok(json!({
                "secret-code": secret_code, // shared by buyer out of band to seller
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
                "amount": response.amount, // this amount will be (ecash in the contract - arbiter fee)
                "state": response.state,
                // secret_code_hash is intentionally omitted to not expose it in the response
            }))
        }
        Command::EscrowClaim {
            escrow_id,
            secret_code,
        } => {
            escrow
                .claim_escrow(escrow_id, response.amount, secret_code)
                .await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved"
            }))
        }
        Command::EscrowDispute { escrow_id } => {
            // the arbiter will take a fee (decided off band)
            escrow.initiate_dispute(escrow_id).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "disputed!"
            }))
            // out of band notification to arbiter give escrow_id to get the
            // contract detail
        }
        Command::EscrowArbiterDecision {
            escrow_id,
            decision,
            arbiter_fee_bps, //
        } => {
            // arbiter will decide the ecash should be given to buyer or seller and change
            // the state of escrow!
            // the arbiter will take a fee (decided off band)
            // decision has 2 values, buyer or seller.
            escrow
                .arbiter_decision(escrow_id, decision, signature, arbiter_fee_bps)
                .await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "arbiter decision made!"
            }))
        }
        Command::BuyerClaim { escrow_id } => {
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
            // the state should be waiting for buyer to claim the ecash as arbiter has
            // decided
            if response.state != EscrowStates::WaitingforBuyerToClaim {
                return Err(EscrowError::ArbiterNotDecided);
            }
            // the amount to be claimed by buyer is the contract amount - arbiter fee
            escrow.buyer_claim(escrow_id, response.amount).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved!"
            }))
        }
        Command::SellerClaim { escrow_id } => {
            let response: ModuleInfo = escrow
                .client_ctx
                .api()
                .request(GET_MODULE_INFO, escrow_id)
                .await?;
            if response.state == EscrowStates::Disputed {
                return Err(EscrowError::EscrowDisputed);
            }
            // the state should be waiting for seller to claim the ecash as arbiter has
            // decided
            if response.state != EscrowStates::WaitingforSellerToClaim {
                return Err(EscrowError::ArbiterNotDecided);
            }
            // the amount to be claimed by seller is the contract amount - arbiter fee
            escrow.seller_claim(escrow_id, response.amount).await?;
            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved!"
            }))
        }
    };

    Ok(res)
}
