#![allow(unused)]
// use bitcoin::hex::DisplayHex;
use bitcoincore_rpc::bitcoin::{Amount, Network, Txid};
use bitcoincore_rpc::{Auth, Client, RpcApi};
use serde::Deserialize;
use serde_json::json;
use std::fs::File;
use std::io::Write;
use std::str::FromStr;
const RPC_URL: &str = "http://127.0.0.1:18443";
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

fn send(rpc: &Client, addr: &str) -> bitcoincore_rpc::Result<String> {
    let args = [
        json!([{addr: 100}]),
        json!(null),
        json!(null),
        json!(null),
        json!(null),
    ];
    #[derive(Deserialize)]
    struct SendResult {
        complete: bool,
        txid: String,
    }
    let send_result = rpc.call::<SendResult>("send", &args)?;
    assert!(send_result.complete);
    Ok(send_result.txid)
}

// TODO: Helper to construct a wallet-scoped RPC client (routes to /wallet/<name>)
fn wallet_client(name: &str) -> bitcoincore_rpc::Result<Client> {
    Client::new(
        &format!("{}/wallet/{}", RPC_URL, name),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )
}

// TODO: Create/Load the wallets. Have logic to optionally create/load them if they
// do not exist or are not loaded already.
fn create_or_load_wallet(rpc: &Client, name: &str) -> bitcoincore_rpc::Result<()> {
    if rpc.create_wallet(name, None, None, None, None).is_ok() { return Ok(()) }
    match rpc.load_wallet(name) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("already loaded") => Ok(()),
        Err(e) => Err(e),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // TODO: Connect to Bitcoin Core RPC
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // TODO: Get blockchain info
    let blockchain_info = rpc.get_blockchain_info()?;
    println!("Blockchain Info: {:?}", blockchain_info);

    // TODO: Create/load wallets named 'Miner' and 'Trader' (case-sensitive)
    create_or_load_wallet(&rpc, "Miner")?;
    create_or_load_wallet(&rpc, "Trader")?;

    let miner_rpc = wallet_client("Miner")?;
    let trader_rpc = wallet_client("Trader")?;

    // TODO: Generate a new address from the Miner wallet with label "Mining Reward"
    let mining_address = miner_rpc
        .get_new_address(Some("Mining Reward"), None)?
        .require_network(bitcoincore_rpc::bitcoin::Network::Regtest)
        .unwrap();

    // TODO: Mine blocks to the Miner address until wallet has a positive balance (use generatetoaddress)
    // Coinbase outputs are subject to a 100-block maturity rule: you cannot spend a coinbase
    // output until it has 100 confirmations. This protects against spending rewards from
    // orphaned blocks. 101 blocks = 1 spendable coinbase, 100 still immature.
    miner_rpc.generate_to_address(101, &mining_address)?;

    // TODO: Print the balance of the Miner wallet
    let balance = miner_rpc.get_balance(None, None)?;
    println!("Miner balance: {} BTC", balance.to_btc());

    // TODO: Load Trader wallet and generate a receiving address labeled "Received"
    let trader_address = trader_rpc
        .get_new_address(Some("Received"), None)?
        .require_network(Network::Regtest)
        .unwrap();

    // TODO: Send 20 BTC from Miner to Trader
    let txid = miner_rpc.send_to_address(
        &trader_address,
        Amount::from_btc(20.0).unwrap(),
        None,
        None,
        None,
        None,
        None,
        None,
    )?;

    // TODO: Fetch the unconfirmed transaction from the mempool and print it (getmempoolentry)
    let mempool_entry =
        rpc.call::<serde_json::Value>("getmempoolentry", &[json!(txid.to_string())])?;
    println!("Mempool entry: {:#?}", mempool_entry);

    // TODO: Grab full verbose tx details while still in mempool.
    // getrawtransaction works for unconfirmed txs without -txindex.
    // Once confirmed, you'd need the block hash — which we don't have yet.
    let raw_tx_json: serde_json::Value =
        rpc.call("getrawtransaction", &[json!(txid.to_string()), json!(true)])?;

    // TODO: Mine 1 block to confirm the transaction
    miner_rpc.generate_to_address(1, &mining_address)?;

    // TODO: Get block hash and height at which the tx was confirmed
    let tx_info = miner_rpc.get_transaction(&txid, Some(true))?;
    let block_hash = tx_info.info.blockhash.unwrap();
    let block_height = tx_info.info.blockheight.unwrap();
    let fee = tx_info.fee.unwrap().to_btc(); // negative: money leaving the wallet

    // TODO: Resolve the input — find the previous output this tx is spending
    let prev_txid_str = raw_tx_json["vin"][0]["txid"].as_str().unwrap();
    let prev_vout_idx = raw_tx_json["vin"][0]["vout"].as_u64().unwrap() as usize;

    // The previous tx is a confirmed coinbase. Without txindex, getrawtransaction
    // requires the block hash. get_transaction on the miner wallet has it.
    let prev_txid = bitcoincore_rpc::bitcoin::Txid::from_str(prev_txid_str)?;
    let prev_tx_wallet_info = miner_rpc.get_transaction(&prev_txid, Some(true))?;
    let prev_block_hash = prev_tx_wallet_info.info.blockhash.unwrap();

    let prev_tx_json: serde_json::Value = miner_rpc.call(
        "getrawtransaction",
        &[
            json!(prev_txid_str),
            json!(true),
            json!(prev_block_hash.to_string()),
        ],
    )?;

    // TODO: Extract input address and amount from the previous output being spent
    let input_address = prev_tx_json["vout"][prev_vout_idx]["scriptPubKey"]["address"]
        .as_str()
        .unwrap()
        .to_string();
    let input_amount = prev_tx_json["vout"][prev_vout_idx]["value"]
        .as_f64()
        .unwrap();

    // TODO: Split output: trader's address is the payment, everything else is change
    let trader_addr_str = trader_address.to_string();
    let mut trader_output_addr = String::new();
    let mut trader_output_amount = 0.0f64;
    let mut change_addr = String::new();
    let mut change_amount = 0.0f64;

    for vout in raw_tx_json["vout"].as_array().unwrap() {
        let addr = vout["scriptPubKey"]["address"].as_str().unwrap_or("");
        let value = vout["value"].as_f64().unwrap_or(0.0);
        if addr == trader_addr_str {
            trader_output_addr = addr.to_string();
            trader_output_amount = value;
        } else if !addr.is_empty() {
            change_addr = addr.to_string();
            change_amount = value;
        }
    }

    // TODO: Write transaction details to ../out.txt in the required format
    let output = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        txid,
        input_address,
        input_amount,
        trader_output_addr,
        trader_output_amount,
        change_addr,
        change_amount,
        fee,
        block_height,
        block_hash,
    );

    let mut file = File::create("../out.txt")?;
    file.write_all(output.as_bytes())?;

    println!("Written to ../out.txt:\n{}", output);

    Ok(())
}
