use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use nostr::{Keys, ToBech32};
use web_sys::{window, Storage};
use std::sync::Arc;
use std::str::FromStr;

mod wallet_db;
use wallet_db::HybridWalletDatabase;

mod mdk_storage;
use mdk_storage::MdkHybridStorage;

use cdk::wallet::{Wallet, WalletBuilder, ReceiveOptions};
use cdk::nuts::{CurrencyUnit, Token};
use cdk::mint_url::MintUrl;

/// Helper function to create a wallet from stored keys and database
async fn create_wallet() -> Result<Wallet, JsValue> {
    // Get Nostr keys from localStorage
    let storage = get_local_storage()?;
    let secret_hex = storage
        .get_item("nostr_secret_key")?
        .ok_or_else(|| JsValue::from_str("No keys found in localStorage"))?;

    let keys = Keys::parse(&secret_hex)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))?;

    // Create seed from Nostr secret key
    let mut seed = [0u8; 64];
    seed[..32].copy_from_slice(keys.secret_key().as_secret_bytes());

    // Create mint URL
    let mint_url = MintUrl::from_str("https://nofees.testnut.cashu.space")
        .map_err(|e| JsValue::from_str(&format!("Invalid mint URL: {}", e)))?;

    // Create hybrid database (loads from localStorage)
    let db = HybridWalletDatabase::new().await?;

    // Build wallet
    let wallet = WalletBuilder::new()
        .mint_url(mint_url)
        .unit(CurrencyUnit::Sat)
        .localstore(Arc::new(db))
        .seed(seed)
        .build()
        .map_err(|e| JsValue::from_str(&format!("Failed to build wallet: {}", e)))?;

    Ok(wallet)
}

// Helper to get localStorage
fn get_local_storage() -> Result<Storage, JsValue> {
    window()
        .ok_or_else(|| JsValue::from_str("No window object"))?
        .local_storage()?
        .ok_or_else(|| JsValue::from_str("No localStorage available"))
}

/// Initialize the library (call this first from JavaScript)
#[wasm_bindgen(start)]
pub fn init() {
    // Set panic hook for better error messages in console
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Generate new Nostr keys and save to localStorage
#[wasm_bindgen]
pub fn generate_keys() -> Result<String, JsValue> {
    let keys = Keys::generate();
    let secret_hex = keys.secret_key().to_secret_hex();

    let storage = get_local_storage()?;
    storage.set_item("nostr_secret_key", &secret_hex)?;

    Ok(keys.public_key().to_bech32().expect("bech32 encoding is infallible"))
}

/// Load existing keys from localStorage, or generate if none exist
#[wasm_bindgen]
pub fn get_or_create_keys() -> Result<String, JsValue> {
    let storage = get_local_storage()?;

    if let Some(secret_hex) = storage.get_item("nostr_secret_key")? {
        // Load existing keys
        let keys = Keys::parse(&secret_hex)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))?;
        Ok(keys.public_key().to_bech32().expect("bech32 encoding is infallible"))
    } else {
        // Generate new keys
        generate_keys()
    }
}

/// Get the npub (public key in bech32 format)
#[wasm_bindgen]
pub fn get_npub() -> Result<String, JsValue> {
    let storage = get_local_storage()?;

    let secret_hex = storage
        .get_item("nostr_secret_key")?
        .ok_or_else(|| JsValue::from_str("No keys found in localStorage"))?;

    let keys = Keys::parse(&secret_hex)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))?;

    Ok(keys.public_key().to_bech32().expect("bech32 encoding is infallible"))
}

/// Clear all stored keys (for testing)
#[wasm_bindgen]
pub fn clear_keys() -> Result<(), JsValue> {
    let storage = get_local_storage()?;
    storage.remove_item("nostr_secret_key")?;
    Ok(())
}

/// Log to browser console (for debugging)
#[wasm_bindgen]
pub fn log(message: &str) {
    web_sys::console::log_1(&JsValue::from_str(message));
}

/// Initialize a real CDK wallet with hybrid in-memory + localStorage storage
/// Returns a Promise that resolves to the initial balance
#[wasm_bindgen]
pub fn init_wallet() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Initializing CDK wallet with hybrid storage...");

            // Create wallet
            let wallet = create_wallet().await?;

            log("Fetching wallet balance...");

            // Get initial balance
            let balance = wallet
                .total_balance()
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to get balance: {}", e)))?;

            log(&format!("✅ Wallet initialized! Balance: {} sats", balance));

            Ok::<u64, JsValue>(u64::from(balance))
        }
        .await;

        result.map(|b| JsValue::from_f64(b as f64))
    })
}

/// Get wallet balance (recreates wallet from localStorage each time)
/// Returns a Promise that resolves to the current balance
#[wasm_bindgen]
pub fn get_balance() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching balance from wallet...");

            // Create wallet (loads from localStorage)
            let wallet = create_wallet().await?;

            // Get balance
            let balance = wallet
                .total_balance()
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to get balance: {}", e)))?;

            log(&format!("Balance: {} sats", balance));

            Ok::<u64, JsValue>(u64::from(balance))
        }
        .await;

        result.map(|b| JsValue::from_f64(b as f64))
    })
}

/// Receive ecash token
/// Returns a Promise that resolves to the amount received
#[wasm_bindgen]
pub fn receive_token(token_str: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("Receiving token: {}", &token_str[..20.min(token_str.len())]));

            // Parse token to validate it
            let _token = Token::from_str(&token_str)
                .map_err(|e| JsValue::from_str(&format!("Invalid token: {}", e)))?;

            log(&format!("Token parsed, attempting to receive..."));

            // Create wallet (loads from localStorage)
            let wallet = create_wallet().await?;

            // Receive the token
            let amount = wallet
                .receive(&token_str, ReceiveOptions::default())
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to receive token: {}", e)))?;

            log(&format!("✅ Received {} sats!", amount));

            Ok::<u64, JsValue>(u64::from(amount))
        }
        .await;

        result.map(|amount| JsValue::from_f64(amount as f64))
    })
}

/// Initialize MDK storage for testing persistence
/// Returns a Promise that resolves when storage is initialized
#[wasm_bindgen]
pub fn init_mdk_storage() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Initializing MDK storage...");

            // Create MDK storage (loads from localStorage if it exists)
            let _storage = MdkHybridStorage::new().await?;

            log("✅ MDK storage initialized!");

            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::UNDEFINED)
    })
}
