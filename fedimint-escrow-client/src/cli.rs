use std::{ffi, iter};

use clap::Parser;
use fedimint_core::Amount;
use fedimint_escrow_common::endpoints::EscrowInfo;
use fedimint_escrow_common::hash256;
use random_string::generate;
use secp256k1::PublicKey;
use serde::Serialize;
use serde_json::json;

use super::EscrowClientModule;
use crate::api::EscrowFederationApi;

#[derive(Parser, Serialize)]
enum Command {
    Create {
        seller_pubkey: PublicKey,
        arbiter_pubkey: PublicKey,
        cost: Amount,             // actual cost of product
        max_arbiter_fee_bps: u16, // maximum arbiter fee in basis points
    },
    Info {
        escrow_id: String,
    },
    Claim {
        escrow_id: String,
        secret_code: String,
    },
    Dispute {
        escrow_id: String,
    },
    ArbiterDecision {
        escrow_id: String,
        decision: String,
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
        Command::Create {
            seller_pubkey,
            arbiter_pubkey,
            cost,
            max_arbiter_fee_bps,
        } => {
            // Create a random escrow id, which will only be known by the buyer, and will be
            // shared to seller or arbiter by the buyer
            let escrow_id: String = generate(
                32,
                "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            );

            // Generate a random secret code
            let secret_code: String = generate(
                32,
                "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
            );

            let secret_code_hash = hash256(secret_code.clone());

            // finalize_and_submit txns to lock ecash by underfunding
            let (_operation_id, _out_point) = escrow
                .create_escrow(
                    cost,
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
        Command::Info { escrow_id } => {
            // get escrow info corresponding to the id from db using federation api
            let escrow_value: EscrowInfo =
                escrow.module_api.get_escrow_info(escrow_id.clone()).await?;

            Ok(json!({
                "buyer_pubkey": escrow_value.buyer_pubkey,
                "seller_pubkey": escrow_value.seller_pubkey,
                "arbiter_pubkey": escrow_value.arbiter_pubkey,
                "amount": escrow_value.amount, // this amount will be (ecash in the contract - arbiter fee)
                "state": escrow_value.state,
            }))
        }
        Command::Claim {
            escrow_id,
            secret_code,
        } => {
            // get escrow info corresponding to the id from db using federation api
            let escrow_value: EscrowInfo =
                escrow.module_api.get_escrow_info(escrow_id.clone()).await?;

            // arbiter fee is 0 in this case!
            escrow
                .claim_escrow(escrow_id.clone(), escrow_value.amount, secret_code)
                .await?;

            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved"
            }))
        }
        Command::Dispute { escrow_id } => {
            // the arbiter will take a fee (decided off band)
            escrow.initiate_dispute(escrow_id.clone()).await?;

            Ok(json!({
                "escrow_id": escrow_id,
                "status": "disputed!"
            }))
            // out of band notification to arbiter give escrow_id to get the
            // contract detail
        }
        Command::ArbiterDecision {
            escrow_id,
            decision,
            arbiter_fee_bps,
        } => {
            // arbiter will decide the ecash should be given to buyer or seller and change
            // the state of escrow!
            // the arbiter will take a fee (decided off band)
            // decision has 2 values, buyer or seller.
            escrow
                .arbiter_decision(escrow_id.clone(), decision, arbiter_fee_bps)
                .await?;

            Ok(json!({
                "escrow_id": escrow_id,
                "status": "arbiter decision made!"
            }))
        }
        Command::BuyerClaim { escrow_id } => {
            // get escrow info corresponding to the id from db using federation api
            let escrow_value: EscrowInfo =
                escrow.module_api.get_escrow_info(escrow_id.clone()).await?;

            // the amount to be claimed by buyer is the contract amount - arbiter fee
            escrow
                .buyer_claim(escrow_id.clone(), escrow_value.amount)
                .await?;

            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved!"
            }))
        }
        Command::SellerClaim { escrow_id } => {
            // get escrow info corresponding to the id from db using federation api
            let escrow_value: EscrowInfo =
                escrow.module_api.get_escrow_info(escrow_id.clone()).await?;

            // the amount to be claimed by seller is the contract amount - arbiter fee
            escrow
                .seller_claim(escrow_id.clone(), escrow_value.amount)
                .await?;

            Ok(json!({
                "escrow_id": escrow_id,
                "status": "resolved!"
            }))
        }
    };

    res
}
