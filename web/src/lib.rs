use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use nostr::{Keys, ToBech32, FromBech32, EventBuilder, Kind, RelayUrl};
use nostr_sdk::{Client, RelayPoolNotification};
use web_sys::{window, Storage};
use std::sync::Arc;
use std::str::FromStr;
use std::time::Duration;
use serde::Serialize;
use once_cell::sync::Lazy;
use tokio::sync::Mutex as TokioMutex;

mod wallet_db;
use wallet_db::HybridWalletDatabase;

mod mdk_storage;
use mdk_storage::MdkHybridStorage;

use cdk::wallet::{Wallet, WalletBuilder, ReceiveOptions};
use cdk::nuts::{CurrencyUnit, Token};
use cdk::mint_url::MintUrl;

use mdk_core::MDK;
use mdk_storage_traits::GroupId;

/// Global storage cache - loaded once per browser session
static STORAGE_CACHE: Lazy<TokioMutex<Option<MdkHybridStorage>>> =
    Lazy::new(|| TokioMutex::new(None));

/// Get or create cached storage instance
async fn get_or_create_storage() -> Result<MdkHybridStorage, JsValue> {
    let mut cache = STORAGE_CACHE.lock().await;

    if let Some(storage) = cache.as_ref() {
        // Return a clone of the cached storage (cheap - uses Arc internally)
        return Ok(storage.clone());
    }

    // First access this session - load from localStorage
    log("📦 Loading storage from localStorage (first access this session)");
    let storage = MdkHybridStorage::new().await?;
    *cache = Some(storage.clone());
    log("✅ Storage cached for session");
    Ok(storage)
}

/// Helper function to create MDK instance
async fn create_mdk() -> Result<MDK<MdkHybridStorage>, JsValue> {
    let storage = get_or_create_storage().await?;
    Ok(MDK::new(storage))
}

/// Helper function to get Nostr keys
fn get_keys() -> Result<Keys, JsValue> {
    let storage = get_local_storage()?;
    let secret_hex = storage
        .get_item("nostr_secret_key")?
        .ok_or_else(|| JsValue::from_str("No keys found in localStorage"))?;

    Keys::parse(&secret_hex)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))
}

/// Helper function to create a Nostr client connected to configured relays
async fn create_connected_client() -> Result<Client, JsValue> {
    let client = Client::default();
    let relays = get_relays_internal()?;
    for relay in &relays {
        if let Ok(url) = RelayUrl::parse(relay) {
            let _ = client.add_relay(url).await;
        }
    }
    client.connect().await;
    Ok(client)
}

/// Get current mint URL from localStorage, or use default
fn get_current_mint_url() -> Result<String, JsValue> {
    let storage = get_local_storage()?;

    // Try to get current mint from localStorage
    if let Ok(Some(mint_url)) = storage.get_item("current_mint_url") {
        return Ok(mint_url);
    }

    // Default mint if none set
    let default_mint = "https://nofees.testnut.cashu.space".to_string();

    // Save default as current mint
    storage.set_item("current_mint_url", &default_mint)?;

    Ok(default_mint)
}

/// Helper function to create a wallet for a specific mint URL
async fn create_wallet_for_mint(mint_url_str: String) -> Result<Wallet, JsValue> {
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

    // Parse the provided mint URL
    let mint_url = MintUrl::from_str(&mint_url_str)
        .map_err(|e| JsValue::from_str(&format!("Invalid mint URL: {}", e)))?;

    // Create hybrid database (loads from localStorage - shared across all mints)
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

/// Helper function to create a wallet from stored keys and database
/// Uses the current mint URL from localStorage
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

    // Get current mint URL from localStorage
    let mint_url_str = get_current_mint_url()?;
    let mint_url = MintUrl::from_str(&mint_url_str)
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
    console_error_panic_hook::set_once();
}

/// Generate new Nostr keys and save to localStorage
/// Also clears MDK state since it's associated with the old identity (keeps wallet)
#[wasm_bindgen]
pub fn generate_keys() -> Result<String, JsValue> {
    let keys = Keys::generate();
    let secret_hex = keys.secret_key().to_secret_hex();

    let storage = get_local_storage()?;

    // Clear MDK state from previous identity (but keep wallet)
    storage.remove_item("mdk_state")?;
    log("Cleared old MDK state for fresh start (wallet preserved)");

    // Save new keys
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

/// Clear all stored keys and MDK state (for testing, keeps wallet)
#[wasm_bindgen]
pub fn clear_keys() -> Result<(), JsValue> {
    let storage = get_local_storage()?;
    storage.remove_item("nostr_secret_key")?;
    storage.remove_item("mdk_state")?;
    log("Cleared Nostr keys and MDK state (wallet preserved)");
    Ok(())
}

// ============================================================================
// Trusted Mints Management
// ============================================================================

/// Get list of trusted mint URLs
/// Returns JSON array of strings
#[wasm_bindgen]
pub fn get_trusted_mints() -> Result<String, JsValue> {
    let storage = get_local_storage()?;

    let mints_json = storage
        .get_item("trusted_mints")?
        .unwrap_or_else(|| "[]".to_string());

    Ok(mints_json)
}

/// Add a mint to the trusted list
/// Returns true if added, false if already in list
#[wasm_bindgen]
pub fn add_trusted_mint(mint_url: String) -> Result<bool, JsValue> {
    let storage = get_local_storage()?;

    // Validate mint URL
    MintUrl::from_str(&mint_url)
        .map_err(|e| JsValue::from_str(&format!("Invalid mint URL: {}", e)))?;

    // Load current list
    let mints_json = storage
        .get_item("trusted_mints")?
        .unwrap_or_else(|| "[]".to_string());

    let mut mints: Vec<String> = serde_json::from_str(&mints_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse trusted mints: {}", e)))?;

    // Check if already in list
    if mints.contains(&mint_url) {
        return Ok(false);
    }

    // Add to list
    mints.push(mint_url);

    // Save back to localStorage
    let updated_json = serde_json::to_string(&mints)
        .map_err(|e| JsValue::from_str(&format!("Failed to serialize mints: {}", e)))?;

    storage.set_item("trusted_mints", &updated_json)?;

    Ok(true)
}

/// Remove a mint from the trusted list
/// Returns true if removed, false if not in list
#[wasm_bindgen]
pub fn remove_trusted_mint(mint_url: String) -> Result<bool, JsValue> {
    let storage = get_local_storage()?;

    // Load current list
    let mints_json = storage
        .get_item("trusted_mints")?
        .unwrap_or_else(|| "[]".to_string());

    let mut mints: Vec<String> = serde_json::from_str(&mints_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse trusted mints: {}", e)))?;

    // Find and remove
    let initial_len = mints.len();
    mints.retain(|m| m != &mint_url);

    if mints.len() == initial_len {
        return Ok(false); // Not found
    }

    // Save back to localStorage
    let updated_json = serde_json::to_string(&mints)
        .map_err(|e| JsValue::from_str(&format!("Failed to serialize mints: {}", e)))?;

    storage.set_item("trusted_mints", &updated_json)?;

    Ok(true)
}

/// Check if a mint URL is in the trusted list
#[wasm_bindgen]
pub fn is_mint_trusted(mint_url: String) -> Result<bool, JsValue> {
    let storage = get_local_storage()?;

    let mints_json = storage
        .get_item("trusted_mints")?
        .unwrap_or_else(|| "[]".to_string());

    let mints: Vec<String> = serde_json::from_str(&mints_json)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse trusted mints: {}", e)))?;

    Ok(mints.contains(&mint_url))
}

/// Set the current mint URL (for wallet operations)
#[wasm_bindgen]
pub fn set_current_mint(mint_url: String) -> Result<(), JsValue> {
    let storage = get_local_storage()?;

    // Validate mint URL
    MintUrl::from_str(&mint_url)
        .map_err(|e| JsValue::from_str(&format!("Invalid mint URL: {}", e)))?;

    storage.set_item("current_mint_url", &mint_url)?;

    Ok(())
}

/// Get the current mint URL
#[wasm_bindgen]
pub fn get_current_mint() -> Result<String, JsValue> {
    get_current_mint_url()
}

/// Log to browser console (for debugging)
#[wasm_bindgen]
pub fn log(message: &str) {
    web_sys::console::log_1(&JsValue::from_str(message));
}

// Default relays if none are configured
const DEFAULT_RELAYS: &[&str] = &[
    "wss://orangesync.tech",
    "wss://nostr.chaima.info",
    "wss://relay.primal.net",
];

/// Internal helper to get relays list (for Rust usage)
fn get_relays_internal() -> Result<Vec<String>, JsValue> {
    let storage = get_local_storage()?;

    match storage.get_item("nostr_relays")? {
        Some(json_str) => {
            serde_json::from_str::<Vec<String>>(&json_str)
                .map_err(|e| JsValue::from_str(&format!("Failed to parse relays: {}", e)))
        }
        None => {
            // Return default relays
            Ok(DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect())
        }
    }
}

/// Get the list of configured relays
/// Returns a Promise that resolves to a JSON array of relay URLs
#[wasm_bindgen]
pub fn get_relays() -> js_sys::Promise {
    future_to_promise(async move {
        let relays = get_relays_internal()?;
        let json = serde_json::to_string(&relays)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize relays: {}", e)))?;
        Ok(JsValue::from_str(&json))
    })
}

/// Add a relay to the configured list (with validation)
/// Returns a Promise that resolves when the relay is added
#[wasm_bindgen]
pub fn add_relay(url: String) -> js_sys::Promise {
    future_to_promise(async move {
        // Validate URL format
        if !url.starts_with("ws://") && !url.starts_with("wss://") {
            return Err(JsValue::from_str("Relay URL must start with ws:// or wss://"));
        }

        let relay_url = RelayUrl::parse(&url)
            .map_err(|e| JsValue::from_str(&format!("Invalid relay URL: {}", e)))?;

        // Test connection (simple validation - timeout not supported in WASM)
        log(&format!("Testing connection to {}...", url));
        let client = Client::default();
        client.add_relay(relay_url.clone()).await
            .map_err(|e| JsValue::from_str(&format!("Failed to add relay: {}", e)))?;

        client.connect().await;

        // Give it a moment to connect, then check status
        wasm_bindgen_futures::JsFuture::from(js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 2000)
                .unwrap();
        })).await.ok();

        // Check if relay is actually connected
        match client.relay(relay_url.clone()).await {
            Ok(relay) => {
                if relay.is_connected() {
                    log(&format!("✓ Successfully connected to {}", url));
                } else {
                    let _ = client.disconnect().await;
                    return Err(JsValue::from_str(&format!("Failed to connect to relay: {}", url)));
                }
            }
            Err(e) => {
                let _ = client.disconnect().await;
                return Err(JsValue::from_str(&format!("Relay error: {}", e)));
            }
        }

        let _ = client.disconnect().await;

        // Add to list
        let mut relays = get_relays_internal()?;

        if relays.contains(&url) {
            return Err(JsValue::from_str("Relay already in list"));
        }

        relays.push(url.clone());

        let storage = get_local_storage()?;
        let json = serde_json::to_string(&relays)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize relays: {}", e)))?;
        storage.set_item("nostr_relays", &json)?;

        log(&format!("✅ Added relay: {}", url));
        Ok(JsValue::from_str("success"))
    })
}

/// Remove a relay from the configured list
/// Returns a Promise that resolves when the relay is removed
#[wasm_bindgen]
pub fn remove_relay(url: String) -> js_sys::Promise {
    future_to_promise(async move {
        let mut relays = get_relays_internal()?;

        let original_len = relays.len();
        relays.retain(|r| r != &url);

        if relays.len() == original_len {
            return Err(JsValue::from_str("Relay not found in list"));
        }

        if relays.is_empty() {
            return Err(JsValue::from_str("Cannot remove last relay"));
        }

        let storage = get_local_storage()?;
        let json = serde_json::to_string(&relays)
            .map_err(|e| JsValue::from_str(&format!("Failed to serialize relays: {}", e)))?;
        storage.set_item("nostr_relays", &json)?;

        log(&format!("✅ Removed relay: {}", url));
        Ok(JsValue::from_str("success"))
    })
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

/// Get balances for all mints in the wallet database
/// Returns a Promise that resolves to JSON array of {mint: string, balance: number, is_trusted: bool}
/// Sorted by descending balance
#[wasm_bindgen]
pub fn get_all_mint_balances() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            use std::collections::HashMap;
            use cdk_common::database::WalletDatabase;
            use cdk::nuts::State;

            log("Fetching balances for all mints...");

            // Create database to access proofs
            let db = HybridWalletDatabase::new().await?;

            // Get all unspent proofs (State::Unspent)
            let proofs = db.get_proofs(None, Some(CurrencyUnit::Sat), Some(vec![State::Unspent]), None)
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to get proofs: {}", e)))?;

            // Group by mint URL and sum amounts
            let mut balances: HashMap<String, u64> = HashMap::new();
            for proof in proofs {
                let mint_str = proof.mint_url.to_string();
                *balances.entry(mint_str).or_insert(0) += u64::from(proof.proof.amount);
            }

            // Convert to sorted JSON array
            #[derive(Serialize)]
            struct MintBalance {
                mint: String,
                balance: u64,
                is_trusted: bool,
            }

            let mut mint_balances: Vec<MintBalance> = balances
                .into_iter()
                .map(|(mint, balance)| {
                    let is_trusted = is_mint_trusted(mint.clone()).unwrap_or(false);
                    MintBalance {
                        mint,
                        balance,
                        is_trusted,
                    }
                })
                .collect();

            // Sort by descending balance
            mint_balances.sort_by(|a, b| b.balance.cmp(&a.balance));

            let json = serde_json::to_string(&mint_balances)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            log(&format!("Found balances for {} mint(s)", mint_balances.len()));

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Parse token information without receiving it
/// Returns a Promise that resolves to JSON with token info including trust status
#[wasm_bindgen]
pub fn parse_token_info(token_str: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            // Parse token
            let token = Token::from_str(&token_str)
                .map_err(|e| JsValue::from_str(&format!("Invalid token: {}", e)))?;

            // Get total amount
            let amount = token.value()
                .map_err(|e| JsValue::from_str(&format!("Failed to get value: {}", e)))?;

            // Get mint URL
            let mint_url = token.mint_url()
                .map_err(|e| JsValue::from_str(&format!("Failed to get mint URL: {}", e)))?;

            // Check if mint is trusted
            let mint_str = mint_url.to_string();
            let is_trusted = is_mint_trusted(mint_str.clone())?;

            // Create JSON response
            #[derive(Serialize)]
            struct TokenInfo {
                amount: u64,
                mint: String,
                is_trusted: bool,
            }

            let info = TokenInfo {
                amount: u64::from(amount),
                mint: mint_str,
                is_trusted,
            };

            let json = serde_json::to_string(&info)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Receive ecash token
/// Returns a Promise that resolves to the amount received
/// Creates a wallet for the token's mint (not the current mint)
#[wasm_bindgen]
pub fn receive_token(token_str: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("Receiving token: {}", &token_str[..20.min(token_str.len())]));

            // Parse token to get its mint URL
            let token = Token::from_str(&token_str)
                .map_err(|e| JsValue::from_str(&format!("Invalid token: {}", e)))?;

            let token_mint_url = token.mint_url()
                .map_err(|e| JsValue::from_str(&format!("Failed to get mint URL: {}", e)))?;

            log(&format!("Token is from mint: {}", token_mint_url));

            // Create wallet for the TOKEN'S mint (not current mint)
            let wallet = create_wallet_for_mint(token_mint_url.to_string()).await?;

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

/// Get all groups from MDK storage
/// Returns a Promise that resolves to a JSON array of groups
#[wasm_bindgen]
pub fn get_groups() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching groups from MDK...");

            // Get current user's public key
            let keys = get_keys()?;
            let current_user_pubkey = keys.public_key();

            let mdk = create_mdk().await?;
            let groups = mdk
                .get_groups()
                .map_err(|e| JsValue::from_str(&format!("Failed to get groups: {}", e)))?;

            log(&format!("Found {} group(s)", groups.len()));

            // Convert to JSON array with member count and admin info
            let groups_json: Vec<_> = groups.iter().map(|g| {
                // Get member count
                let member_count = mdk.get_members(&g.mls_group_id)
                    .ok()
                    .map(|members| members.len())
                    .unwrap_or(0);

                // Check if current user is an admin
                let is_admin = g.admin_pubkeys.contains(&current_user_pubkey);

                // Convert admin pubkeys to npubs
                let admin_npubs: Vec<String> = g.admin_pubkeys.iter()
                    .filter_map(|pk| pk.to_bech32().ok())
                    .collect();

                serde_json::json!({
                    "id": hex::encode(g.mls_group_id.as_slice()),
                    "name": g.name,
                    "description": g.description,
                    "image_hash": g.image_hash.map(|h| hex::encode(h)),
                    "last_message_at": g.last_message_at.map(|t| t.as_u64()),
                    "member_count": member_count,
                    "is_admin": is_admin,
                    "admin_npubs": admin_npubs,
                })
            }).collect();

            let json = serde_json::to_string(&groups_json)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Fetch Welcome events from Nostr relays and process them with MDK
/// Returns a Promise that resolves to the number of Welcome events processed
#[wasm_bindgen]
pub fn fetch_welcome_events() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching Welcome events from Nostr relays...");

            // Get our keys
            let keys = get_keys()?;
            let pubkey = keys.public_key();

            // Create client and connect to relays
            let client = create_connected_client().await?;

            // Step 1: Get our KeyPackage event IDs that we have private keys for
            log("Finding our KeyPackage events...");
            let kp_filter = nostr::Filter::new()
                .kind(Kind::Custom(443))
                .author(pubkey);

            let kp_events = client.fetch_events(kp_filter, Duration::from_secs(5)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackages: {}", e)))?;

            let kp_event_ids: Vec<String> = kp_events.iter().map(|e| e.id.to_hex()).collect();
            log(&format!("Found {} KeyPackage(s) on relays: {:?}", kp_event_ids.len(), kp_event_ids));

            if kp_event_ids.is_empty() {
                log("⚠️ No KeyPackages found. Create a new KeyPackage to receive invites.");
                return Ok::<u32, JsValue>(0);
            }

            // Step 2: Get Welcome events that reference our KeyPackages
            log("Querying relays for Welcome events...");

            let filter = nostr::Filter::new()
                .kind(Kind::Custom(444));

            let all_welcomes = client.fetch_events(filter, Duration::from_secs(10)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch Welcome events: {}", e)))?;

            let total_welcomes = all_welcomes.len();

            // Filter to only Welcomes that reference our KeyPackages
            let events: Vec<_> = all_welcomes.into_iter().filter(|event| {
                event.tags.iter().any(|tag| {
                    let kind = tag.kind();
                    if kind.as_str() == "e" {
                        if let Some(event_id) = tag.content() {
                            return kp_event_ids.iter().any(|kp_id| kp_id == event_id);
                        }
                    }
                    false
                })
            }).collect();

            log(&format!("Found {} Welcome event(s) for us (out of {} total)", events.len(), total_welcomes));

            // Disconnect from relays
            let _ = client.disconnect().await;

            // Process each Welcome event with MDK
            let mdk = create_mdk().await?;
            let mut processed = 0;

            for event in events {
                log(&format!("Processing Welcome event: {}", event.id.to_hex()));

                // Log which KeyPackage this Welcome references
                let referenced_kp: Vec<String> = event.tags.iter()
                    .filter(|tag| tag.kind().as_str() == "e")
                    .filter_map(|tag| tag.content().map(|s| s.to_string()))
                    .collect();
                log(&format!("  References KeyPackage(s): {:?}", referenced_kp));

                // Log which pubkeys this Welcome is addressed to (p tags)
                let addressed_to: Vec<String> = event.tags.iter()
                    .filter(|tag| tag.kind().as_str() == "p")
                    .filter_map(|tag| tag.content())
                    .filter_map(|pk_hex| {
                        nostr::PublicKey::from_hex(pk_hex)
                            .ok()
                            .and_then(|pk| pk.to_bech32().ok())
                    })
                    .collect();
                if !addressed_to.is_empty() {
                    log(&format!("  Addressed to npub(s): {:?}", addressed_to));
                } else {
                    log("  No p tags found");
                }

                log(&format!("  Our npub: {}", pubkey.to_bech32().expect("valid bech32")));

                // Check if we already have this welcome in storage
                match mdk.get_welcome(&event.id) {
                    Ok(Some(_)) => {
                        log("  ⚠ Welcome already in storage, skipping");
                        continue;
                    }
                    Ok(None) => {
                        // Not in storage yet, proceed
                    }
                    Err(e) => {
                        log(&format!("  ⚠ Error checking welcome storage: {}", e));
                        continue;
                    }
                }

                // Try to extract the rumor from the event
                // Welcome events might be gift-wrapped (kind 1059) or direct (kind 444)
                match nostr::nips::nip59::UnwrappedGift::from_gift_wrap(&keys, &event).await {
                    Ok(unwrapped) => {
                        // Successfully unwrapped - process the rumor
                        log("  Unwrapped gift-wrapped Welcome");
                        match mdk.process_welcome(&event.id, &unwrapped.rumor) {
                            Ok(_) => {
                                log("  ✓ Welcome processed successfully");
                                processed += 1;
                            }
                            Err(e) => {
                                log(&format!("  ✗ Failed to process Welcome: {}", e));
                            }
                        }
                    }
                    Err(_) => {
                        // Not gift-wrapped - process as direct kind 444 event
                        log("  Processing as direct (non-gift-wrapped) Welcome");

                        // Log event details for debugging
                        log(&format!("    Event author: {}", event.pubkey.to_hex()));
                        log(&format!("    Event kind: {}", event.kind.as_u16()));
                        log(&format!("    Event tags: {:?}", event.tags.len()));
                        log(&format!("    Content length: {} bytes", event.content.len()));
                        log(&format!("    Content preview: {}...", &event.content[..event.content.len().min(100)]));

                        // Convert Event to UnsignedEvent
                        let rumor = nostr::UnsignedEvent {
                            id: None,
                            pubkey: event.pubkey,
                            created_at: event.created_at,
                            kind: event.kind,
                            tags: event.tags.clone(),
                            content: event.content.clone(),
                        };

                        log("  Calling MDK.process_welcome()...");
                        match mdk.process_welcome(&event.id, &rumor) {
                            Ok(_) => {
                                log("  ✓ Welcome processed successfully");
                                processed += 1;
                            }
                            Err(e) => {
                                log(&format!("  ✗ Failed to process Welcome: {}", e));
                            }
                        }
                    }
                }
            }

            log(&format!("✅ Processed {} Welcome event(s)", processed));

            Ok::<u32, JsValue>(processed)
        }
        .await;

        result.map(|count| JsValue::from_f64(count as f64))
    })
}

/// Generate KeyPackage and wait for group invite (all in one flow)
/// Returns a Promise that resolves to the group name when joined
#[wasm_bindgen]
pub fn create_keypackage_and_wait_for_invite() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("🔑 Creating KeyPackage and waiting for invite...");

            // Get keys
            let keys = get_keys()?;
            let pubkey = keys.public_key();

            // Create MDK (this instance will stay alive for the whole flow)
            let mdk = create_mdk().await?;

            // Step 1: Create KeyPackage
            log("Creating KeyPackage...");
            let relays = get_relays_internal()?;
            let relay_urls: Vec<RelayUrl> = relays
                .iter()
                .filter_map(|r| RelayUrl::parse(r).ok())
                .collect();

            let (key_package_hex, tags) = mdk
                .create_key_package_for_event(&pubkey, relay_urls)
                .map_err(|e| JsValue::from_str(&format!("Failed to create KeyPackage: {}", e)))?;

            // Step 2: Build and sign event
            let event = EventBuilder::new(Kind::Custom(443), key_package_hex)
                .tags(tags.to_vec())
                .sign_with_keys(&keys)
                .map_err(|e| JsValue::from_str(&format!("Failed to sign event: {}", e)))?;

            let kp_event_id = event.id;
            log(&format!("KeyPackage event ID: {}", kp_event_id.to_hex()));

            // Step 3: Connect to relays and start listening for Welcomes BEFORE publishing
            let client = create_connected_client().await?;

            // Subscribe to Welcomes that reference our KeyPackage
            log("Starting Welcome subscription...");
            let filter = nostr::Filter::new()
                .kind(Kind::Custom(444))
                .since(nostr::Timestamp::now());  // Only new Welcomes from now on

            client.subscribe(filter, None).await
                .map_err(|e| JsValue::from_str(&format!("Failed to subscribe: {}", e)))?;

            // Step 4: Now publish KeyPackage (Welcomes can arrive while publishing!)
            log("Publishing KeyPackage to relays...");
            let send_result = client.send_event(&event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish: {}", e)))?;

            log(&format!("✅ KeyPackage published! Your npub: {}", pubkey.to_bech32().expect("valid bech32")));
            for relay_url in send_result.success.iter() {
                log(&format!("  ✓ {} accepted", relay_url));
            }
            for (relay_url, error) in send_result.failed.iter() {
                log(&format!("  ✗ {} rejected: {}", relay_url, error));
            }
            log("⏳ Waiting for group invite...");

            // Step 5: Listen for Welcome via subscription (may already be here!)
            let mut notifications = client.notifications();
            log("Listening for Welcome messages...");

            loop {
                // Wait for next notification from subscription
                match notifications.recv().await {
                    Ok(notification) => {
                        if let nostr_sdk::RelayPoolNotification::Event { relay_url, event: welcome_event, .. } = notification {
                            log(&format!("📩 Received event: {} from relay: {}", welcome_event.id.to_hex(), relay_url));

                            // Check if this Welcome references our KeyPackage
                            let references_our_kp = welcome_event.tags.iter().any(|tag| {
                                tag.kind().as_str() == "e" &&
                                tag.content().map(|c| c == kp_event_id.to_hex()).unwrap_or(false)
                            });

                            if !references_our_kp {
                                log("  Not for our KeyPackage, continuing...");
                                continue;
                            }

                            log(&format!("  ✅ This Welcome is for our KeyPackage! (from {})", relay_url));

                            // Process the Welcome
                            log(&format!("Processing Welcome: {} (delivered by {})", welcome_event.id.to_hex(), relay_url));

                            // Convert to UnsignedEvent (compute the ID)
                            let mut rumor = nostr::UnsignedEvent {
                                id: None,
                                pubkey: welcome_event.pubkey,
                                created_at: welcome_event.created_at,
                                kind: welcome_event.kind,
                                tags: welcome_event.tags.clone(),
                                content: welcome_event.content.clone(),
                            };
                            // Ensure the ID is set
                            let _ = rumor.id();

                            // Process Welcome
                            let welcome = mdk.process_welcome(&welcome_event.id, &rumor)
                                .map_err(|e| JsValue::from_str(&format!("Failed to process Welcome: {}", e)))?;

                            log(&format!("Welcome processed! Group: {}", welcome.group_name));

                            // Accept Welcome (join the group)
                            mdk.accept_welcome(&welcome)
                                .map_err(|e| JsValue::from_str(&format!("Failed to accept Welcome: {}", e)))?;

                            log(&format!("✅ Joined group: {}", welcome.group_name));

                            // Disconnect and return
                            let _ = client.disconnect().await;

                            return Ok::<String, JsValue>(welcome.group_name);
                        }
                    }
                    Err(e) => {
                        log(&format!("Error receiving notification: {:?}", e));
                        // Continue waiting
                    }
                }
            }
        }
        .await;

        result.map(|name| JsValue::from_str(&name))
    })
}

/// Create a new group and invite members
/// Returns a Promise that resolves to the group ID
#[wasm_bindgen]
pub fn create_group_with_members(name: String, description: String, member_npubs_json: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📝 Creating group: {}", name));

            // Parse member npubs
            let member_npubs: Vec<String> = serde_json::from_str(&member_npubs_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid npubs JSON: {}", e)))?;

            // Get our keys
            let keys = get_keys()?;
            let our_pubkey = keys.public_key();

            // Fetch KeyPackages for each member
            log(&format!("Fetching KeyPackages for {} member(s)...", member_npubs.len()));

            let client = create_connected_client().await?;

            let mut key_package_events = Vec::new();
            let mut admin_pubkeys = vec![our_pubkey]; // Creator is always admin

            for npub in &member_npubs {
                log(&format!("  Fetching KeyPackage for {}...", &npub[..16]));

                // Parse npub to get public key
                let pubkey = nostr::PublicKey::from_bech32(npub)
                    .map_err(|e| JsValue::from_str(&format!("Invalid npub {}: {}", npub, e)))?;

                admin_pubkeys.push(pubkey);

                // Query for their most recent KeyPackage (kind 443)
                let filter = nostr::Filter::new()
                    .kind(Kind::Custom(443))
                    .author(pubkey)
                    .limit(10);  // Get last 10, we'll pick the newest

                let events = client.fetch_events(filter, Duration::from_secs(10)).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackages for {}: {}", npub, e)))?;

                if events.is_empty() {
                    return Err(JsValue::from_str(&format!("No KeyPackage found for {}", npub)));
                }

                // Get the newest KeyPackage
                let newest = events.iter()
                    .max_by_key(|e| e.created_at)
                    .unwrap();

                log(&format!("    ✓ Found KeyPackage: {}", newest.id.to_hex()));
                key_package_events.push(newest.clone());
            }

            // Create group config
            use mdk_core::prelude::*;
            let relays = get_relays_internal()?;
            let relay_urls: Vec<RelayUrl> = relays
                .iter()
                .filter_map(|r| RelayUrl::parse(r).ok())
                .collect();

            let config = NostrGroupConfigData::new(
                name.clone(),
                description,
                None,  // image
                None,  // banner
                None,  // website
                relay_urls,
                admin_pubkeys,
            );

            // Create MDK and create the group
            log("Creating group with MDK...");
            let mdk = create_mdk().await?;

            let group_result = mdk.create_group(&our_pubkey, key_package_events, config)
                .map_err(|e| JsValue::from_str(&format!("Failed to create group: {}", e)))?;

            let group_id = hex::encode(group_result.group.mls_group_id.as_slice());
            log(&format!("✅ Group created! ID: {}", &group_id[..16]));

            // Publish Welcome messages to each invited member
            log(&format!("Publishing {} Welcome message(s)...", group_result.welcome_rumors.len()));

            for welcome_unsigned in group_result.welcome_rumors {
                // Sign the UnsignedEvent
                let welcome_event = welcome_unsigned.sign(&keys).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to sign Welcome: {}", e)))?;

                let send_result = client.send_event(&welcome_event).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to send Welcome: {}", e)))?;

                log("Publishing Welcome message:");
                for relay_url in send_result.success.iter() {
                    log(&format!("  ✓ {} accepted Welcome", relay_url));
                }
                for (relay_url, error) in send_result.failed.iter() {
                    log(&format!("  ✗ {} rejected Welcome: {}", relay_url, error));
                }
            }

            log(&format!("✅ All Welcome messages published!"));

            // Disconnect
            let _ = client.disconnect().await;

            Ok::<String, JsValue>(group_id)
        }
        .await;

        result.map(|group_id| JsValue::from_str(&group_id))
    })
}

/// Invite a member to an existing group by their npub
/// Returns a Promise that resolves when the invite is sent
#[wasm_bindgen]
pub fn invite_member_to_group(group_id_hex: String, member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("👋 Inviting {} to group {}", &member_npub[..16], &group_id_hex[..16]));

            // Parse npub to get public key
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            // Parse group ID
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID: {}", e)))?;
            let group_id = mdk_core::prelude::GroupId::from_slice(&group_id_bytes);

            // Fetch member's KeyPackage
            log(&format!("Fetching KeyPackage for {}...", &member_npub[..16]));

            let client = create_connected_client().await?;

            let filter = nostr::Filter::new()
                .kind(Kind::Custom(443))
                .author(member_pubkey)
                .limit(10);

            let events = client.fetch_events(filter, Duration::from_secs(10)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackage: {}", e)))?;

            if events.is_empty() {
                return Err(JsValue::from_str(&format!("No KeyPackage found for {}. They may need to create one first.", &member_npub[..16])));
            }

            // Get the newest KeyPackage
            let newest = events.iter()
                .max_by_key(|e| e.created_at)
                .unwrap();

            log(&format!("  ✓ Found KeyPackage: {}", newest.id.to_hex()));

            // Get our keys
            let keys = get_keys()?;

            // Create MDK
            let mdk = create_mdk().await?;

            // Step 1: Send "Inviting..." message to existing members (before adding new member)
            log(&format!("Notifying existing members about invitation..."));
            let inviting_message = format!("Inviting {} to the group...", &member_npub);

            let rumor = nostr::UnsignedEvent {
                id: None,
                pubkey: keys.public_key(),
                created_at: nostr::Timestamp::now(),
                kind: Kind::GiftWrap,
                tags: nostr::Tags::new(),
                content: inviting_message,
            };

            let message_event = mdk.create_message(&group_id, rumor)
                .map_err(|e| JsValue::from_str(&format!("Failed to create 'inviting' message: {}", e)))?;

            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge 'inviting' message commit: {}", e)))?;

            client.send_event(&message_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to send 'inviting' message: {}", e)))?;

            log("✅ Existing members notified");

            // Step 2: Add member to group
            log("Adding member to group...");
            let invite_result = mdk.add_members(&group_id, &[newest.clone()])
                .map_err(|e| JsValue::from_str(&format!("Failed to add member: {}", e)))?;

            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge commit: {}", e)))?;

            client.send_event(&invite_result.evolution_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish evolution: {}", e)))?;

            // Step 3: Publish Welcome message
            if let Some(welcome_rumors) = invite_result.welcome_rumors {
                log(&format!("Publishing Welcome message to {}...", &member_npub[..16]));

                for welcome_unsigned in welcome_rumors {
                    let welcome_event = welcome_unsigned.sign(&keys).await
                        .map_err(|e| JsValue::from_str(&format!("Failed to sign Welcome: {}", e)))?;

                    let send_result = client.send_event(&welcome_event).await
                        .map_err(|e| JsValue::from_str(&format!("Failed to send Welcome: {}", e)))?;

                    for relay_url in send_result.success.iter() {
                        log(&format!("  ✓ {} accepted Welcome", relay_url));
                    }
                    for (relay_url, error) in send_result.failed.iter() {
                        log(&format!("  ✗ {} rejected Welcome: {}", relay_url, error));
                    }
                }

                log(&format!("✅ Welcome sent to {}!", &member_npub[..16]));
            }

            // Step 4: Promote new member to admin
            log("Adding new member as admin...");

            // Get current group data
            let group = mdk.get_group(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get group: {}", e)))?
                .ok_or_else(|| JsValue::from_str("Group not found"))?;

            // Add the new member to admins
            let mut new_admins: Vec<nostr::PublicKey> = group.admin_pubkeys.into_iter().collect();
            new_admins.push(member_pubkey);

            // Update group data with new admin list
            use mdk_core::prelude::NostrGroupDataUpdate;
            let update = NostrGroupDataUpdate {
                admins: Some(new_admins),
                ..Default::default()
            };

            let update_result = mdk.update_group_data(&group_id, update)
                .map_err(|e| JsValue::from_str(&format!("Failed to update admins: {}", e)))?;

            // Merge the update commit BEFORE publishing
            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge admin update: {}", e)))?;

            // Publish the update evolution event
            client.send_event(&update_result.evolution_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish admin update: {}", e)))?;

            log("✅ Member added as admin!");

            // Step 5: Send final confirmation message to everyone (including new member)
            log("Sending confirmation to all members...");
            let confirmation_message = format!("{} joined the group and was promoted to admin", &member_npub);

            // Create message rumor
            let rumor = nostr::UnsignedEvent {
                id: None,
                pubkey: keys.public_key(),
                created_at: nostr::Timestamp::now(),
                kind: Kind::GiftWrap,
                tags: nostr::Tags::new(),
                content: confirmation_message,
            };

            // Create encrypted message
            let message_event = mdk.create_message(&group_id, rumor)
                .map_err(|e| JsValue::from_str(&format!("Failed to create confirmation message: {}", e)))?;

            // Merge pending commit BEFORE publishing
            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge confirmation commit: {}", e)))?;

            // Publish confirmation message
            client.send_event(&message_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to send confirmation: {}", e)))?;

            log("✅ Confirmation sent to all members!");

            // Disconnect
            let _ = client.disconnect().await;

            Ok::<JsValue, JsValue>(JsValue::from_str("success"))
        }
        .await;

        result
    })
}

/// Check for and auto-accept any pending Welcome messages from MDK storage
/// Returns a Promise that resolves to the number of groups joined
#[wasm_bindgen]
pub fn process_pending_welcomes() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Checking for pending Welcome messages...");

            // Get MDK instance
            let mdk = create_mdk().await?;

            // Get all pending welcomes and auto-accept them
            let pending_welcomes = mdk.get_pending_welcomes()
                .map_err(|e| JsValue::from_str(&format!("Failed to get pending welcomes: {}", e)))?;

            log(&format!("Found {} pending welcome(s) to accept", pending_welcomes.len()));

            let mut accepted = 0;
            for welcome in pending_welcomes {
                log(&format!("Auto-accepting welcome for group: {}", hex::encode(welcome.mls_group_id.as_slice())));

                match mdk.accept_welcome(&welcome) {
                    Ok(_) => {
                        log("  ✓ Welcome accepted, joined group!");
                        accepted += 1;
                    }
                    Err(e) => {
                        log(&format!("  ✗ Failed to accept: {}", e));
                    }
                }
            }

            if accepted > 0 {
                log(&format!("✅ Joined {} new group(s)", accepted));
            } else {
                log("No new groups to join");
            }

            Ok::<u32, JsValue>(accepted)
        }
        .await;

        result.map(|count| JsValue::from_f64(count as f64))
    })
}

/// Send a message to a group
/// Returns a Promise that resolves when the message is sent
#[wasm_bindgen]
pub fn send_message_to_group(group_id_hex: String, message_content: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📤 Sending message to group {}", &group_id_hex[..16]));
            log(&format!("  Message content: {}", message_content));

            // Get keys
            let keys = get_keys()?;
            let pubkey = keys.public_key();
            log(&format!("  Sender npub: {}", pubkey.to_bech32().expect("valid bech32")));

            // Decode group ID from hex
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID hex: {}", e)))?;
            let group_id = GroupId::from_slice(&group_id_bytes);
            log(&format!("  Decoded group ID: {} bytes", group_id_bytes.len()));

            // Create MDK
            log("  Creating MDK instance...");
            let mdk = create_mdk().await?;
            log("  ✓ MDK instance created");

            // Verify group exists
            log("  Checking if group exists...");
            let group = mdk.get_group(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get group: {}", e)))?
                .ok_or_else(|| JsValue::from_str("Group not found"))?;
            log(&format!("  ✓ Group found: {}", &group.name));

            // Create message rumor
            log("  Creating message rumor...");
            let rumor = nostr::UnsignedEvent {
                id: None,
                pubkey,
                created_at: nostr::Timestamp::now(),
                kind: Kind::GiftWrap,
                tags: nostr::Tags::new(),
                content: message_content.clone(),
            };
            log("  ✓ Message rumor created");

            // Create encrypted message
            log("  Encrypting message with MLS...");
            let message_event = mdk.create_message(&group_id, rumor)
                .map_err(|e| JsValue::from_str(&format!("Failed to create message: {}", e)))?;
            log(&format!("  ✓ Message encrypted, event ID: {}", message_event.id.to_hex()));

            // Publish to relays
            log("  Connecting to relays...");
            let client = create_connected_client().await?;
            log("  ✓ Connected to relays");

            // Merge pending commit to finalize our state BEFORE publishing
            log("  Finalizing message state...");
            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge commit: {}", e)))?;
            log("  ✓ State finalized");

            log("  Publishing message event...");
            client.send_event(&message_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to send event: {}", e)))?;
            log("  ✓ Message event published");

            // Disconnect
            let _ = client.disconnect().await;
            log("✅ Message sent successfully!");

            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::undefined())
    })
}

/// Get messages for a group from storage
/// Returns a Promise that resolves to a JSON array of messages
#[wasm_bindgen]
pub fn get_messages_for_group(group_id_hex: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            // Decode group ID from hex
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID hex: {}", e)))?;
            let group_id = GroupId::from_slice(&group_id_bytes);

            // Get storage
            let storage = MdkHybridStorage::new().await?;

            // Get messages using the GroupStorage trait
            use mdk_storage_traits::groups::GroupStorage;
            let messages = storage.messages(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get messages: {}", e)))?;

            // Convert messages to JSON
            #[derive(Serialize)]
            struct MessageJson {
                id: String,
                pubkey: String,
                content: String,
                created_at: u64,
                state: String,
            }

            let messages_json: Vec<MessageJson> = messages.iter().map(|msg| {
                MessageJson {
                    id: msg.id.to_hex(),
                    pubkey: msg.pubkey.to_hex(),
                    content: msg.content.clone(),
                    created_at: msg.created_at.as_u64(),
                    state: msg.state.to_string(),
                }
            }).collect();

            let json = serde_json::to_string(&messages_json)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Message JSON structure for JavaScript callback
#[derive(Serialize)]
struct MessageCallback {
    id: String,
    pubkey: String,
    content: String,
    created_at: u64,
    state: String,
}

/// Subscribe to group messages and call a JavaScript callback for each new message
/// The callback will receive a JSON object with message details
#[wasm_bindgen]
pub fn subscribe_to_group_messages(group_id_hex: String, callback: js_sys::Function) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📡 Subscribing to messages for group {}", &group_id_hex[..16]));

            // Decode group ID
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID hex: {}", e)))?;
            let group_id = GroupId::from_slice(&group_id_bytes);

            // Get the group to find its nostr_group_id (used in event tags)
            let mdk = create_mdk().await?;
            let group = mdk.get_group(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get group: {}", e)))?
                .ok_or_else(|| JsValue::from_str("Group not found"))?;

            let nostr_group_id_hex = hex::encode(group.nostr_group_id);
            log(&format!("  Filtering by nostr_group_id: {}", &nostr_group_id_hex[..16]));

            // Create client and connect to relays
            let client = create_connected_client().await?;
            log("  ✓ Connected to relays");

            // Subscribe to MLS group messages (kind 445) filtered by this specific group
            // Optimization: If we have message history, only fetch recent messages (last 10 min + buffer)
            let filter = if let Some(last_msg_time) = group.last_message_at {
                let ten_minutes = 600; // 10 minutes in seconds
                let since = nostr::Timestamp::from(last_msg_time.as_u64().saturating_sub(ten_minutes));

                log(&format!("  Subscribing since {} (last_message_at - 10 min)", since.as_u64()));

                nostr::Filter::new()
                    .kind(Kind::MlsGroupMessage)
                    .custom_tag(nostr::SingleLetterTag::lowercase(nostr::Alphabet::H), nostr_group_id_hex)
                    .since(since)
            } else {
                log("  First join - fetching all history");

                nostr::Filter::new()
                    .kind(Kind::MlsGroupMessage)
                    .custom_tag(nostr::SingleLetterTag::lowercase(nostr::Alphabet::H), nostr_group_id_hex)
            };

            log("  Subscribing to MLS group messages (kind 445)...");
            client.subscribe(filter, None).await
                .map_err(|e| JsValue::from_str(&format!("Failed to subscribe: {}", e)))?;
            log("  ✓ Subscribed successfully");

            // Spawn a background task to listen for notifications
            wasm_bindgen_futures::spawn_local(async move {
                log("  📻 Starting notification listener...");
                let mut notifications = client.notifications();

                while let Ok(notification) = notifications.recv().await {
                    if let RelayPoolNotification::Event { relay_url, event, .. } = notification {
                        log(&format!("  📩 Received event: {} from relay: {}", event.id.to_hex(), relay_url));

                        // Create MDK instance and process the message
                        match create_mdk().await {
                            Ok(mdk) => {
                                match mdk.process_message(&event) {
                                    Ok(result) => {
                                        use mdk_core::prelude::MessageProcessingResult;
                                        if let MessageProcessingResult::ApplicationMessage(msg) = result {
                                            log(&format!("  ✅ Application message: '{}' (from {})", msg.content, relay_url));
                                            log(&format!("     Message group ID: {}", hex::encode(msg.mls_group_id.as_slice())));
                                            log(&format!("     Target group ID: {}", hex::encode(group_id.as_slice())));

                                            // Check if this message belongs to the current group
                                            if msg.mls_group_id == group_id {
                                                log(&format!("  🎯 Message matches current group! (delivered by {})", relay_url));

                                                // Prepare callback data
                                                let msg_data = MessageCallback {
                                                    id: msg.id.to_hex(),
                                                    pubkey: msg.pubkey.to_hex(),
                                                    content: msg.content,
                                                    created_at: msg.created_at.as_u64(),
                                                    state: msg.state.to_string(),
                                                };

                                                // Call the JavaScript callback
                                                if let Ok(js_value) = serde_wasm_bindgen::to_value(&msg_data) {
                                                    match callback.call1(&JsValue::NULL, &js_value) {
                                                        Ok(_) => log("  ✅ Callback invoked successfully"),
                                                        Err(e) => log(&format!("  ❌ Callback failed: {:?}", e)),
                                                    }
                                                } else {
                                                    log("  ❌ Failed to serialize message to JS value");
                                                }
                                            } else {
                                                log("  ⏭️  Message is for a different group, skipping");
                                            }
                                        } else {
                                            log(&format!("  ℹ️  Non-application message: {:?}", result));
                                        }
                                    }
                                    Err(e) => {
                                        use mdk_core::error::Error;

                                        // Check if this is an epoch conflict
                                        if matches!(e, Error::ProcessMessageWrongEpoch) {
                                            log(&format!("  ⚠️  EPOCH CONFLICT DETECTED: Another group member's action was processed first"));
                                            log(&format!("     Event ID: {}", event.id.to_hex()));
                                            log(&format!("     Your local state may have diverged from the group"));

                                            // Show user-friendly modal
                                            if let Some(window) = web_sys::window() {
                                                let _ = window.alert_with_message(
                                                    "⚠️ Group Conflict Detected\n\n\
                                                    Another group member performed an action at the same time as you.\n\
                                                    Their action was processed first.\n\n\
                                                    Please try your action again (send message, invite member, etc.)."
                                                );
                                            }
                                        } else {
                                            log(&format!("  ⚠️  Failed to process message: {}", e));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log(&format!("  ⚠️  Failed to create MDK: {:?}", e));
                            }
                        }
                    }
                }
            });

            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::undefined())
    })
}

