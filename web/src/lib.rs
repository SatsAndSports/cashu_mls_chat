use wasm_bindgen::prelude::*;
use nostr::{Keys, ToBech32};
use web_sys::{window, Storage};

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
