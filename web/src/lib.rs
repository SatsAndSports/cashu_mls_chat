use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use nostr::{Keys, ToBech32};
use web_sys::{window, Storage};
use std::sync::Arc;
use std::str::FromStr;

mod wallet_db;
use wallet_db::HybridWalletDatabase;

use cdk::wallet::WalletBuilder;
use cdk::nuts::CurrencyUnit;
use cdk::mint_url::MintUrl;

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

            log("Creating hybrid wallet database...");

            // Create hybrid database (in-memory + localStorage snapshots)
            let db = HybridWalletDatabase::new().await?;

            log("Building CDK wallet...");

            // Build wallet
            let wallet = WalletBuilder::new()
                .mint_url(mint_url)
                .unit(CurrencyUnit::Sat)
                .localstore(Arc::new(db))
                .seed(seed)
                .build()
                .map_err(|e| JsValue::from_str(&format!("Failed to build wallet: {}", e)))?;

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

/// Get wallet balance
/// Returns a Promise that resolves to the current balance
#[wasm_bindgen]
pub fn get_balance() -> js_sys::Promise {
    future_to_promise(async move {
        // For now, just return 0 since we need to store the wallet instance
        // This will be improved later when we add proper wallet state management
        Ok(JsValue::from_f64(0.0))
    })
}
