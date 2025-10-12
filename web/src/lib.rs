use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use nostr::{Keys, ToBech32, FromBech32, EventBuilder, Kind, RelayUrl, SecretKey, Filter};
use nostr_sdk::{Client, RelayPoolNotification};
use web_sys::{window, Storage};
use std::sync::Arc;
use std::str::FromStr;
use std::time::Duration;
use std::collections::HashSet;
use serde::{Serialize, Deserialize};
use once_cell::sync::Lazy;
use tokio::sync::Mutex as TokioMutex;

mod wallet_db;
use wallet_db::HybridWalletDatabase;

mod mdk_storage;
use mdk_storage::{MdkHybridStorage, SharedMdkStorage};

use cdk::wallet::{Wallet, WalletBuilder, ReceiveOptions};
use cdk::nuts::{CurrencyUnit, Token};
use cdk::mint_url::MintUrl;
use cdk::amount::SplitTarget;

use mdk_core::MDK;
use mdk_storage_traits::GroupId;

/// Global storage cache - Arc-wrapped singleton loaded once per browser session
static STORAGE_CACHE: Lazy<TokioMutex<Option<Arc<MdkHybridStorage>>>> =
    Lazy::new(|| TokioMutex::new(None));

/// Global wallet database - singleton loaded once per browser session
/// This database is shared by ALL wallet instances (across all mints)
static WALLET_DB: Lazy<TokioMutex<Option<HybridWalletDatabase>>> =
    Lazy::new(|| TokioMutex::new(None));

/// Get or create the singleton wallet database
/// Returns a clone (cheap - Arc internally) that shares the same state
async fn get_or_create_wallet_db() -> Result<HybridWalletDatabase, JsValue> {
    let mut cache = WALLET_DB.lock().await;

    if let Some(db) = cache.as_ref() {
        // Return a clone (cheap - just clones the Arc pointer)
        return Ok(db.clone());
    }

    // First access this session - load from localStorage
    log("📦 Loading wallet database from localStorage (first access this session)");
    let db = HybridWalletDatabase::new().await?;
    *cache = Some(db.clone());
    log("✅ Wallet database cached for session");
    Ok(db)
}

/// Get or create cached storage instance (Arc-wrapped for sharing)
async fn get_or_create_storage() -> Result<SharedMdkStorage, JsValue> {
    let mut cache = STORAGE_CACHE.lock().await;

    if let Some(storage) = cache.as_ref() {
        // Return a clone of the Arc (cheap - just increments refcount)
        return Ok(SharedMdkStorage::new(Arc::clone(storage)));
    }

    // First access this session - load from localStorage and wrap in Arc
    log("📦 Loading storage from localStorage (first access this session)");
    let storage = Arc::new(MdkHybridStorage::new().await?);
    *cache = Some(Arc::clone(&storage));
    log("✅ Storage cached for session");
    Ok(SharedMdkStorage::new(storage))
}

/// Helper function to create MDK instance
async fn create_mdk() -> Result<MDK<SharedMdkStorage>, JsValue> {
    let storage = get_or_create_storage().await?;
    Ok(MDK::new(storage))
}

/// Clear the in-memory storage cache (call this when changing identity)
#[wasm_bindgen]
pub async fn clear_storage_cache() {
    let mut cache = STORAGE_CACHE.lock().await;
    *cache = None;
    log("🗑️  Cleared in-memory storage cache");
}

/// Save storage if there are any pending changes
/// This is meant to be called periodically from JavaScript (e.g., every 30 seconds)
#[wasm_bindgen]
pub fn save_storage() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save storage: {:?}", e)))?;
            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::undefined())
    })
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

    // Get singleton database (shared across all mints and all wallet instances)
    let db = get_or_create_wallet_db().await?;

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

    // Get singleton database (shared across all mints and all wallet instances)
    let db = get_or_create_wallet_db().await?;

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

/// Helper for ordered event subscriptions
/// Collects historical events until EOSE, sorts by created_at (oldest first),
/// processes them in order, then continues with real-time events
async fn subscribe_with_ordered_history<F, Fut>(
    client: &Client,
    filter: Filter,
    event_handler: F,
) -> Result<(), JsValue>
where
    F: Fn(Box<nostr::Event>) -> Fut + Clone + 'static,
    Fut: std::future::Future<Output = Result<(), JsValue>>,
{
    // Subscribe to filter
    client.subscribe(filter.clone(), None).await
        .map_err(|e| JsValue::from_str(&format!("Failed to subscribe: {}", e)))?;

    let mut notifications = client.notifications();

    // Track which relays have sent EOSE
    let mut eose_relays: HashSet<String> = HashSet::new();
    let mut historical_events: Vec<Box<nostr::Event>> = Vec::new();
    let mut processed_historical = false;

    while let Ok(notification) = notifications.recv().await {
        match notification {
            RelayPoolNotification::Event { event, .. } => {
                if !processed_historical {
                    // Still collecting historical events
                    historical_events.push(event);
                } else {
                    // Real-time event - process immediately
                    event_handler(event).await?;
                }
            }
            RelayPoolNotification::Message { relay_url, message } => {
                // Check for EOSE message using Debug format (EOSE is RelayMessage::EndOfStoredEvents)
                let msg_str = format!("{:?}", message);
                if msg_str.contains("EndOfStoredEvents") {
                    eose_relays.insert(relay_url.to_string());
                    log(&format!("  EOSE from {} ({} total)", relay_url, eose_relays.len()));

                    // Once we have EOSE from first relay, process historical events
                    if !processed_historical && eose_relays.len() >= 1 {
                        log(&format!("  Sorting {} historical events by created_at...", historical_events.len()));

                        // Sort by created_at (oldest first)
                        historical_events.sort_by_key(|e| e.created_at);

                        // Process sorted historical events
                        for event in historical_events.drain(..) {
                            event_handler(event).await?;
                        }

                        processed_historical = true;
                        log("  ✓ Historical events processed, switching to real-time mode");
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
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

#[wasm_bindgen]
pub fn get_nsec() -> Result<String, JsValue> {
    let storage = get_local_storage()?;

    let secret_hex = storage
        .get_item("nostr_secret_key")?
        .ok_or_else(|| JsValue::from_str("No keys found in localStorage"))?;

    let keys = Keys::parse(&secret_hex)
        .map_err(|e| JsValue::from_str(&format!("Failed to parse keys: {}", e)))?;

    Ok(keys.secret_key().to_bech32().expect("bech32 encoding is infallible"))
}

/// Import an existing nsec and set it as the current identity
#[wasm_bindgen]
pub fn import_nsec(nsec: &str) -> Result<(), JsValue> {
    // Parse the nsec to validate it and convert to hex
    let secret_key = SecretKey::from_bech32(nsec)
        .map_err(|e| JsValue::from_str(&format!("Invalid nsec: {}", e)))?;

    let keys = Keys::new(secret_key);

    // Store in localStorage as hex
    let storage = get_local_storage()?;
    storage.set_item("nostr_secret_key", &keys.secret_key().to_secret_hex())
        .map_err(|e| JsValue::from_str(&format!("Failed to store keys: {:?}", e)))?;

    log(&format!("Imported identity: {}", keys.public_key().to_hex()));
    Ok(())
}

/// Get the public key in hex format
#[wasm_bindgen]
pub fn get_pubkey_hex() -> Result<String, JsValue> {
    let keys = get_keys()?;
    Ok(keys.public_key().to_hex())
}

/// Fetch profile metadata (Kind 0) for a given npub
/// Returns JSON string with the metadata or null if not found
#[wasm_bindgen]
pub fn fetch_profile_metadata(npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        // Decode npub to get hex pubkey
        let pubkey = nostr::PublicKey::from_bech32(&npub)
            .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

        // Create and connect client
        let client = create_connected_client().await?;

        // Fetch Kind 0 events for this pubkey
        let filter = Filter::new()
            .kind(Kind::Metadata)
            .author(pubkey)
            .limit(1);

        let events = client.fetch_events(filter, Duration::from_secs(5)).await
            .map_err(|e| JsValue::from_str(&format!("Failed to fetch events: {}", e)))?;

        // Disconnect
        let _ = client.disconnect().await;

        if events.is_empty() {
            return Ok(JsValue::NULL);
        }

        // Return the most recent event's content (which is already JSON)
        let event_vec: Vec<_> = events.iter().collect();
        if let Some(event) = event_vec.first() {
            Ok(JsValue::from_str(&event.content))
        } else {
            Ok(JsValue::NULL)
        }
    })
}

/// Publish profile metadata (Kind 0 event)
/// metadata_json should be a JSON string with fields like: name, display_name, about, picture, nip05
#[wasm_bindgen]
pub fn publish_profile_metadata(metadata_json: String) -> js_sys::Promise {
    future_to_promise(async move {
        let keys = get_keys()?;

        // Parse JSON into Metadata struct
        let metadata: nostr::Metadata = serde_json::from_str(&metadata_json)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse metadata: {}", e)))?;

        // Create Kind 0 event
        let event = EventBuilder::metadata(&metadata)
            .sign_with_keys(&keys)
            .map_err(|e| JsValue::from_str(&format!("Failed to sign event: {}", e)))?;

        // Create and connect client
        let client = create_connected_client().await?;

        // Publish event
        client.send_event(&event).await
            .map_err(|e| JsValue::from_str(&format!("Failed to publish metadata: {}", e)))?;

        // Disconnect
        let _ = client.disconnect().await;

        log("✅ Profile metadata published");
        Ok(JsValue::from_str("success"))
    })
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
/// Validates the mint by connecting and fetching keysets
/// Returns a Promise that resolves to true if added, false if already in list
#[wasm_bindgen]
pub fn add_trusted_mint(mint_url: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("🔍 Validating mint: {}", mint_url));

            // Validate mint URL format
            let _mint_url_parsed = MintUrl::from_str(&mint_url)
                .map_err(|e| {
                    log(&format!("❌ Invalid URL format: {}", e));
                    JsValue::from_str(&format!("Invalid mint URL: {}", e))
                })?;

            log("📡 Creating wallet and connecting to mint...");

            // Try to connect to the mint and fetch its info
            // This requires actual network communication with the mint
            let wallet = create_wallet_for_mint(mint_url.clone()).await
                .map_err(|e| {
                    log(&format!("❌ Failed to create wallet: {:?}", e));
                    e
                })?;

            log("🔎 Fetching mint info from network...");

            // Fetch mint info - this makes an actual HTTP request to the mint
            // Will fail if mint is unreachable or not a valid Cashu mint
            let mint_info_option = wallet.fetch_mint_info()
                .await
                .map_err(|e| {
                    log(&format!("❌ fetch_mint_info failed: {:?}", e));
                    JsValue::from_str(&format!("Failed to connect to mint (not a valid Cashu mint): {}", e))
                })?;

            // Check if we actually got mint info
            let mint_info = mint_info_option
                .ok_or_else(|| {
                    log("❌ Mint returned no info (unreachable or not a valid Cashu mint)");
                    JsValue::from_str("Failed to connect to mint: No mint info returned. This is not a valid Cashu mint or is unreachable.")
                })?;

            log(&format!("✅ Mint info received: {:?}", mint_info));

            // Now add to trusted list
            let storage = get_local_storage()?;

            // Load current list
            let mints_json = storage
                .get_item("trusted_mints")?
                .unwrap_or_else(|| "[]".to_string());

            let mut mints: Vec<String> = serde_json::from_str(&mints_json)
                .map_err(|e| JsValue::from_str(&format!("Failed to parse trusted mints: {}", e)))?;

            // Check if already in list
            if mints.contains(&mint_url) {
                log("ℹ️ Mint already in trusted list");
                return Ok::<bool, JsValue>(false);
            }

            // Add to list
            mints.push(mint_url.clone());

            // Save back to localStorage
            let updated_json = serde_json::to_string(&mints)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize mints: {}", e)))?;

            storage.set_item("trusted_mints", &updated_json)?;

            log(&format!("✅ Mint added to trusted list: {}", mint_url));

            Ok(true)
        }
        .await;

        result.map(|added| JsValue::from_bool(added))
    })
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

/// Get transaction history from wallet database
/// Returns a Promise that resolves to JSON array of transaction objects
/// Sorted by most recent first
#[wasm_bindgen]
pub fn get_transaction_history() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            use cdk_common::database::WalletDatabase;

            log("Fetching transaction history...");

            // Get singleton database to access transactions
            let db = get_or_create_wallet_db().await?;

            // Get all transactions (no filters)
            let mut transactions = db.list_transactions(None, None, None)
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to get transactions: {}", e)))?;

            // Sort by timestamp (most recent first)
            transactions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

            // Convert to JSON
            #[derive(Serialize)]
            struct TransactionInfo {
                amount: u64,
                direction: String,
                mint: String,
                timestamp: u64,
                unit: String,
            }

            let tx_infos: Vec<TransactionInfo> = transactions
                .into_iter()
                .map(|tx| TransactionInfo {
                    amount: u64::from(tx.amount),
                    direction: format!("{:?}", tx.direction),
                    mint: tx.mint_url.to_string(),
                    timestamp: tx.timestamp,
                    unit: format!("{:?}", tx.unit),
                })
                .collect();

            let json = serde_json::to_string(&tx_infos)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            log(&format!("Found {} transactions", tx_infos.len()));

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
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

            // Get singleton database to access proofs
            let db = get_or_create_wallet_db().await?;

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

            // Extract secret kind and data from token
            let mut secret_kind: Option<String> = None;
            let mut secret_data: Option<String> = None;
            let mut secret_npub: Option<String> = None;

            // Get proofs from the token to check secret kind
            if let Ok(proofs) = token.proofs(&[]) {
                if let Some(first_proof) = proofs.first() {
                    // The secret is a JSON array like ["P2PK", {"data": "...", ...}] or just a plain string
                    let secret_str = first_proof.secret.to_string();

                    // Try to parse as JSON array
                    if let Ok(secret_json) = serde_json::from_str::<serde_json::Value>(&secret_str) {
                        if let Some(arr) = secret_json.as_array() {
                            if let Some(first_elem) = arr.first() {
                                if let Some(kind_str) = first_elem.as_str() {
                                    secret_kind = Some(kind_str.to_string());
                                }
                            }
                            // If kind is P2PK, extract data from second element
                            if secret_kind.as_deref() == Some("P2PK") && arr.len() >= 2 {
                                if let Some(data_obj) = arr[1].as_object() {
                                    if let Some(data_val) = data_obj.get("data") {
                                        if let Some(data_str) = data_val.as_str() {
                                            secret_data = Some(data_str.to_string());

                                            // Try to convert hex pubkey to npub
                                            if let Ok(pubkey_bytes) = hex::decode(data_str) {
                                                if pubkey_bytes.len() == 33 {
                                                    // Convert compressed secp256k1 pubkey to x-only (Nostr format)
                                                    if let Ok(secp_pk) = nostr::secp256k1::PublicKey::from_slice(&pubkey_bytes) {
                                                        let (x_only, _parity) = secp_pk.x_only_public_key();
                                                        if let Ok(nostr_pk) = nostr::PublicKey::from_slice(x_only.serialize().as_ref()) {
                                                            secret_npub = Some(nostr_pk.to_bech32().unwrap());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Create JSON response
            #[derive(Serialize)]
            struct TokenInfo {
                amount: u64,
                mint: String,
                is_trusted: bool,
                #[serde(skip_serializing_if = "Option::is_none")]
                secret_kind: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                secret_data: Option<String>,
                #[serde(skip_serializing_if = "Option::is_none")]
                secret_npub: Option<String>,
            }

            let info = TokenInfo {
                amount: u64::from(amount),
                mint: mint_str,
                is_trusted,
                secret_kind,
                secret_data,
                secret_npub,
            };

            let json = serde_json::to_string(&info)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Send ecash tokens
/// Returns a Promise that resolves to the token string
#[wasm_bindgen]
pub fn send_ecash(amount: u64) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            use cdk::wallet::SendOptions;

            log(&format!("Creating token for {} sats", amount));

            // Create wallet (uses current mint)
            let wallet = create_wallet().await?;

            // Prepare send
            let prepared = wallet
                .prepare_send(cdk::Amount::from(amount), SendOptions::default())
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to prepare send: {}", e)))?;

            // Confirm and create token
            let token = prepared
                .confirm(None)
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to create token: {}", e)))?;

            let token_str = token.to_string();

            log(&format!("✅ Created token: {} sats", amount));

            Ok::<String, JsValue>(token_str)
        }
        .await;

        result.map(|token| JsValue::from_str(&token))
    })
}

/// Send ecash with P2PK - creates a token locked to recipient's public key
/// Returns the token string
#[wasm_bindgen]
pub fn send_ecash_p2pk(amount: u64, recipient_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            use cdk::nuts::SpendingConditions;

            log(&format!("Creating P2PK token for {} sats to {}", amount, &recipient_npub[..16]));

            // Parse recipient npub to get public key
            let recipient_pubkey = nostr::PublicKey::from_bech32(&recipient_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            // Convert Nostr pubkey (32 bytes) to secp256k1 compressed format (33 bytes)
            // Nostr uses x-only pubkeys, we need to convert to full secp256k1 pubkey
            let pubkey_bytes = recipient_pubkey.to_bytes();

            // Create secp256k1 XOnlyPublicKey from the 32-byte Nostr pubkey
            use nostr::secp256k1::XOnlyPublicKey;
            let x_only = XOnlyPublicKey::from_slice(&pubkey_bytes)
                .map_err(|e| JsValue::from_str(&format!("Failed to parse x-only pubkey: {}", e)))?;

            // Convert to full public key (assumes even parity, which is standard for Nostr)
            let full_pubkey = nostr::secp256k1::PublicKey::from_x_only_public_key(x_only, nostr::secp256k1::Parity::Even);

            // Serialize to compressed format (33 bytes)
            let compressed_bytes = full_pubkey.serialize();

            // Convert to CDK PublicKey
            let p2pk_pubkey = cdk::nuts::PublicKey::from_slice(&compressed_bytes)
                .map_err(|e| JsValue::from_str(&format!("Failed to convert pubkey: {}", e)))?;

            // Create P2PK spending conditions
            let spending_conditions = SpendingConditions::new_p2pk(p2pk_pubkey, None);

            // Create wallet (uses current mint)
            let wallet = create_wallet().await?;

            // For P2PK, we must swap ALL proofs to apply the spending conditions
            // Using prepare_send doesn't work because it may send proofs directly without swapping
            // So we use swap_from_unspent directly
            log("Swapping from unspent with P2PK conditions...");
            let proofs = wallet
                .swap_from_unspent(
                    cdk::Amount::from(amount),
                    Some(spending_conditions),
                    false,  // include_fees
                )
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to swap with P2PK: {}", e)))?;

            // Create token from the swapped proofs
            use cdk::nuts::Token;
            let token = Token::new(
                wallet.mint_url.clone(),
                proofs,
                None,  // memo
                wallet.unit.clone(),
            );

            let token_str = token.to_string();

            // Verify the token has P2PK by checking the first proof's secret
            if let Ok(proofs) = token.proofs(&[]) {
                if let Some(first_proof) = proofs.first() {
                    let secret_str = first_proof.secret.to_string();
                    log(&format!("Token secret: {}", secret_str));
                    if !secret_str.contains("P2PK") {
                        log("⚠️ WARNING: Token does not contain P2PK in secret!");
                    }
                }
            }

            log(&format!("✅ Created P2PK token: {} sats locked to {}", amount, &recipient_npub[..16]));

            Ok::<String, JsValue>(token_str)
        }
        .await;

        result.map(|token| JsValue::from_str(&token))
    })
}

/// Receive ecash token
/// Returns a Promise that resolves to the amount received
/// Creates a wallet for the token's mint (not the current mint)
/// Automatically handles P2PK tokens by signing with the user's Nostr key
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

            // Get Nostr keys for P2PK signing (if token is P2PK-locked)
            let storage = window().unwrap().local_storage().unwrap().unwrap();
            let secret_hex = storage.get_item("nostr_secret_key").unwrap()
                .ok_or_else(|| JsValue::from_str("No Nostr key found. Please generate keys first."))?;

            // Parse secret key from hex
            let secret_key = SecretKey::from_str(&secret_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid secret key: {}", e)))?;

            // Convert Nostr secret key bytes to CDK format
            let secret_key_bytes = secret_key.as_secret_bytes();
            let cdk_secret_key = cdk::nuts::SecretKey::from_slice(secret_key_bytes)
                .map_err(|e| JsValue::from_str(&format!("Failed to convert secret key: {}", e)))?;

            // Create wallet for the TOKEN'S mint (not current mint)
            let wallet = create_wallet_for_mint(token_mint_url.to_string()).await?;

            // Receive the token with P2PK signing key
            let receive_options = ReceiveOptions {
                p2pk_signing_keys: vec![cdk_secret_key],
                ..Default::default()
            };

            let amount = wallet
                .receive(&token_str, receive_options)
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to receive token: {}", e)))?;

            log(&format!("✅ Received {} sats!", amount));

            Ok::<u64, JsValue>(u64::from(amount))
        }
        .await;

        result.map(|amount| JsValue::from_f64(amount as f64))
    })
}

/// Decode a Lightning invoice to extract amount, description, and fee
/// Returns JSON with: { amount_msat, description, fee_sats }
#[wasm_bindgen]
pub fn decode_lightning_invoice(invoice: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            use cdk_common::lightning_invoice::Bolt11Invoice;

            let parsed = Bolt11Invoice::from_str(&invoice)
                .map_err(|e| JsValue::from_str(&format!("Invalid invoice: {}", e)))?;

            let amount_msat = parsed.amount_milli_satoshis()
                .ok_or_else(|| JsValue::from_str("Invoice has no amount"))?;

            let description = match parsed.description() {
                cdk_common::lightning_invoice::Bolt11InvoiceDescriptionRef::Direct(desc) => desc.to_string(),
                cdk_common::lightning_invoice::Bolt11InvoiceDescriptionRef::Hash(_) => String::new(),
            };

            // Get fee estimate from melt quote
            let wallet = create_wallet().await?;
            let quote = wallet
                .melt_quote(invoice.clone(), None)
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to get fee quote: {}", e)))?;

            let fee_sats = u64::from(quote.fee_reserve);

            let result = serde_json::json!({
                "amount_msat": amount_msat,
                "description": description,
                "fee_sats": fee_sats,
                "quote_id": quote.id
            });

            Ok::<String, JsValue>(result.to_string())
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Pay a Lightning invoice using a melt quote ID
/// Returns JSON with: { preimage }
#[wasm_bindgen]
pub fn pay_lightning_invoice_with_quote(quote_id: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("Paying Lightning invoice with quote {}...", quote_id));

            // Create wallet for current mint
            let wallet = create_wallet().await?;

            // Pay the invoice using the quote
            log("Melting tokens to pay invoice...");
            let melt_response = wallet
                .melt(&quote_id)
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to pay invoice: {}", e)))?;

            let preimage = melt_response.preimage
                .ok_or_else(|| JsValue::from_str("No preimage returned"))?;

            log(&format!("✅ Payment successful! Preimage: {}", preimage));

            let result = serde_json::json!({
                "preimage": preimage
            });

            Ok::<String, JsValue>(result.to_string())
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Create a Lightning invoice (mint quote) to receive sats
/// Returns JSON with: { invoice, quote_id, mint_url }
#[wasm_bindgen]
pub fn create_lightning_invoice(mint_url: String, amount: u64, description: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("Creating Lightning invoice for {} sats on mint {}...", amount, mint_url));

            // Create wallet for selected mint
            let wallet = create_wallet_for_mint(mint_url.clone()).await?;

            // Create mint quote
            let quote = wallet
                .mint_quote(cdk::Amount::from(amount), Some(description.clone()))
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to create mint quote: {}", e)))?;

            log(&format!("✅ Invoice created: {}", &quote.request[..20.min(quote.request.len())]));

            let result = serde_json::json!({
                "invoice": quote.request,
                "quote_id": quote.id,
                "mint_url": mint_url
            });

            Ok::<String, JsValue>(result.to_string())
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Check mint quote status and mint tokens if paid
/// Returns JSON with: { paid: bool, amount?: number }
#[wasm_bindgen]
pub fn check_mint_quote(mint_url: String, quote_id: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("Checking mint quote {} status...", quote_id));

            // Create wallet for the mint
            let wallet = create_wallet_for_mint(mint_url).await?;

            // Check quote status
            let quote_status = wallet
                .mint_quote_state(&quote_id)
                .await
                .map_err(|e| JsValue::from_str(&format!("Failed to check quote status: {}", e)))?;

            log(&format!("Quote status - State: {:?}", quote_status.state));

            // Check if paid
            use cdk::nuts::MintQuoteState;
            if quote_status.state == MintQuoteState::Paid {
                log("Quote is paid! Minting tokens...");

                // Mint the tokens
                let proofs = wallet
                    .mint(&quote_id, SplitTarget::default(), None)
                    .await
                    .map_err(|e| JsValue::from_str(&format!("Failed to mint tokens: {}", e)))?;

                // Calculate total amount from proofs
                let total_amount: u64 = proofs.iter().map(|p| u64::from(p.amount)).sum();

                log(&format!("✅ Minted {} sats", total_amount));

                let result = serde_json::json!({
                    "paid": true,
                    "amount": total_amount
                });

                Ok::<String, JsValue>(result.to_string())
            } else {
                log("Quote not paid yet");

                let result = serde_json::json!({
                    "paid": false
                });

                Ok::<String, JsValue>(result.to_string())
            }
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
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
                // Get members
                let members = mdk.get_members(&g.mls_group_id)
                    .ok()
                    .unwrap_or_default();

                let member_count = members.len();

                // Convert member pubkeys to npubs
                let member_npubs: Vec<String> = members.iter()
                    .filter_map(|pk| pk.to_bech32().ok())
                    .collect();

                // Check if current user is an admin
                let is_admin = g.admin_pubkeys.contains(&current_user_pubkey);

                // Convert admin pubkeys to npubs
                let admin_npubs: Vec<String> = g.admin_pubkeys.iter()
                    .filter_map(|pk| pk.to_bech32().ok())
                    .collect();

                serde_json::json!({
                    "id": hex::encode(g.mls_group_id.as_slice()),
                    "nostr_group_id": hex::encode(g.nostr_group_id),
                    "name": g.name,
                    "description": g.description,
                    "image_hash": g.image_hash.map(|h| hex::encode(h)),
                    "last_message_at": g.last_message_at.map(|t| t.as_u64()),
                    "member_count": member_count,
                    "member_npubs": member_npubs,
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

/// Create and publish a KeyPackage (passive mode - returns immediately)
/// Returns Promise resolving to JSON: { event_id, created_at }
#[wasm_bindgen]
pub fn create_and_publish_keypackage() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("🔑 Creating and publishing KeyPackage...");

            // Get keys
            let keys = get_keys()?;
            let pubkey = keys.public_key();

            // Get storage first so we can save it after creating KeyPackage
            let storage = get_or_create_storage().await?;

            // Create MDK with the storage
            let mdk = MDK::new(storage.clone());

            // Create KeyPackage
            log("Creating KeyPackage...");
            let relays = get_relays_internal()?;
            let relay_urls: Vec<RelayUrl> = relays
                .iter()
                .filter_map(|r| RelayUrl::parse(r).ok())
                .collect();

            let (key_package_hex, tags) = mdk
                .create_key_package_for_event(&pubkey, relay_urls)
                .map_err(|e| JsValue::from_str(&format!("Failed to create KeyPackage: {}", e)))?;

            log("✓ KeyPackage created");

            // Explicitly save the storage to persist the KeyPackage private key
            // This must be done BEFORE publishing, so the private key is available for later Welcome processing
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save MDK storage: {:?}", e)))?;
            log("✓ KeyPackage private key saved to storage");

            // Build and sign event
            let event = EventBuilder::new(Kind::Custom(443), key_package_hex)
                .tags(tags.to_vec())
                .sign_with_keys(&keys)
                .map_err(|e| JsValue::from_str(&format!("Failed to sign event: {}", e)))?;

            let kp_event_id = event.id.to_hex();
            let created_at = event.created_at.as_u64();
            log(&format!("KeyPackage event ID: {}", kp_event_id));

            // Connect to relays and publish
            let client = create_connected_client().await?;
            log("Publishing KeyPackage to relays...");
            let send_result = client.send_event(&event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish: {}", e)))?;

            log(&format!("✅ KeyPackage published!"));
            for relay_url in send_result.success.iter() {
                log(&format!("  ✓ {} accepted", relay_url));
            }
            for (relay_url, error) in send_result.failed.iter() {
                log(&format!("  ✗ {} rejected: {}", relay_url, error));
            }

            // Disconnect
            let _ = client.disconnect().await;

            // Return event ID, timestamp, and relay results as JSON
            #[derive(Serialize)]
            struct RelayResult {
                url: String,
                success: bool,
                error: Option<String>,
            }

            #[derive(Serialize)]
            struct KeyPackageResult {
                event_id: String,
                created_at: u64,
                relays: Vec<RelayResult>,
            }

            let mut relay_results = Vec::new();

            // Add successful relays
            for relay_url in send_result.success.iter() {
                relay_results.push(RelayResult {
                    url: relay_url.to_string(),
                    success: true,
                    error: None,
                });
            }

            // Add failed relays
            for (relay_url, error) in send_result.failed.iter() {
                relay_results.push(RelayResult {
                    url: relay_url.to_string(),
                    success: false,
                    error: Some(error.to_string()),
                });
            }

            let result = KeyPackageResult {
                event_id: kp_event_id,
                created_at,
                relays: relay_results,
            };

            let json = serde_json::to_string(&result)
                .map_err(|e| JsValue::from_str(&format!("JSON serialization error: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Delete a KeyPackage by publishing a Kind 5 (deletion) event
#[wasm_bindgen]
pub fn delete_keypackage(event_id: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("🗑️  Deleting KeyPackage: {}", event_id));

            // Get keys
            let keys = get_keys()?;

            // Parse the event ID
            let event_id_obj = nostr::EventId::from_hex(&event_id)
                .map_err(|e| JsValue::from_str(&format!("Invalid event ID: {}", e)))?;

            // Create Kind 5 (deletion) event
            let deletion_event = EventBuilder::new(Kind::EventDeletion, "KeyPackage consumed")
                .tag(nostr::Tag::event(event_id_obj))
                .sign_with_keys(&keys)
                .map_err(|e| JsValue::from_str(&format!("Failed to sign deletion event: {}", e)))?;

            // Connect to relays and publish
            let client = create_connected_client().await?;
            let send_result = client.send_event(&deletion_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish deletion: {}", e)))?;

            log(&format!("✅ Kind 5 (delete) published for KeyPackage {}", event_id.chars().take(16).collect::<String>()));
            for relay_url in send_result.success.iter() {
                log(&format!("  ✓ {} accepted deletion", relay_url));
            }

            // Disconnect
            let _ = client.disconnect().await;

            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::undefined())
    })
}

/// DEBUG: Fetch all Kind 444 Welcome events for debugging
/// Returns JSON array of events with their tags
#[wasm_bindgen]
pub fn debug_fetch_welcome_events() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("🔍 DEBUG: Fetching Welcome events addressed to us (Kind 444)...");

            // Get our pubkey to filter by p tag
            let keys = get_keys()?;
            let our_pubkey = keys.public_key();

            let client = create_connected_client().await?;

            // Fetch Kind 444 events addressed to us via p tag
            let filter = nostr::Filter::new()
                .kind(Kind::Custom(444))
                .pubkey(our_pubkey)  // Filter by p tag
                .limit(50);

            let events = client.fetch_events(filter, Duration::from_secs(5)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch events: {}", e)))?;

            log(&format!("  Found {} Welcome events", events.len()));

            // Convert events to serializable format
            #[derive(Serialize)]
            struct DebugEvent {
                id: String,
                pubkey: String,
                created_at: u64,
                content_len: usize,
                tags: Vec<DebugTag>,
            }

            #[derive(Serialize)]
            struct DebugTag {
                kind: String,
                content: Option<String>,
            }

            let debug_events: Vec<DebugEvent> = events.iter().map(|e| {
                DebugEvent {
                    id: e.id.to_hex(),
                    pubkey: e.pubkey.to_hex(),
                    created_at: e.created_at.as_u64(),
                    content_len: e.content.len(),
                    tags: e.tags.iter().map(|tag| {
                        DebugTag {
                            kind: tag.kind().as_str().to_string(),
                            content: tag.content().map(|s| s.to_string()),
                        }
                    }).collect(),
                }
            }).collect();

            let json = serde_json::to_string(&debug_events)
                .map_err(|e| JsValue::from_str(&format!("JSON serialization error: {}", e)))?;

            client.disconnect().await;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Process a Welcome event by fetching it from relays and joining the group
/// Returns JSON: { group_id, group_name, kp_event_id }
#[wasm_bindgen]
pub fn process_welcome_event(welcome_event_id: String, kp_event_id: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("🎉 Processing Welcome event: {}...", &welcome_event_id[..16]));

            // Parse event ID
            let event_id = nostr::EventId::from_hex(&welcome_event_id)
                .map_err(|e| JsValue::from_str(&format!("Invalid event ID: {}", e)))?;

            // Fetch the Welcome event from relays
            log("  Fetching Welcome event from relays...");
            let client = create_connected_client().await?;

            let filter = nostr::Filter::new()
                .kind(Kind::Custom(444))
                .id(event_id);

            let events = client.fetch_events(filter, Duration::from_secs(5)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch Welcome event: {}", e)))?;

            if events.is_empty() {
                return Err(JsValue::from_str("Welcome event not found on relays"));
            }

            let welcome_event = events.into_iter().next().unwrap();
            log(&format!("  ✓ Found Welcome event: {}", welcome_event.id.to_hex()));

            // Convert to UnsignedEvent for MDK processing
            let mut rumor = nostr::UnsignedEvent {
                id: None,
                pubkey: welcome_event.pubkey,
                created_at: welcome_event.created_at,
                kind: welcome_event.kind,
                tags: welcome_event.tags.clone(),
                content: welcome_event.content.clone(),
            };
            rumor.ensure_id();

            // Create MDK and process Welcome
            log("  Processing Welcome with MDK...");
            let mdk = create_mdk().await?;

            // Try to process the Welcome
            // Note: We handle "already processed" errors below, so no need to pre-check
            let welcome = match mdk.process_welcome(&welcome_event.id, &rumor) {
                Ok(w) => w,
                Err(e) => {
                    let error_msg = format!("{}", e);
                    log(&format!("  ❌ MDK error: {}", error_msg));

                    // Check if this is because we already processed it
                    if error_msg.contains("missing welcome") || error_msg.contains("already processed") {
                        log("  ℹ️ This Welcome may have already been processed in a previous session");
                        log("  Checking if we're already in the group...");

                        // We can't determine the group from a failed process_welcome
                        // Return an error indicating it's already processed
                        return Err(JsValue::from_str("Welcome already processed or KeyPackage missing"));
                    }

                    return Err(JsValue::from_str(&format!("Failed to process Welcome: {}", e)));
                }
            };

            let group_id = hex::encode(welcome.mls_group_id.as_slice());
            let group_name = welcome.group_name.clone();
            log(&format!("  ✓ Welcome processed! Group: {}", group_name));

            // Accept Welcome (join the group)
            log("  Accepting Welcome (joining group)...");
            mdk.accept_welcome(&welcome)
                .map_err(|e| JsValue::from_str(&format!("Failed to accept Welcome: {}", e)))?;

            // Explicitly save after accepting Welcome (critical operation)
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save after accept_welcome: {:?}", e)))?;

            log(&format!("✅ Successfully joined group: {}", group_name));

            // Disconnect
            client.disconnect().await;

            // Return result as JSON
            #[derive(Serialize)]
            struct WelcomeResult {
                group_id: String,
                group_name: String,
                kp_event_id: String,
            }

            let result = WelcomeResult {
                group_id,
                group_name,
                kp_event_id,
            };

            let json = serde_json::to_string(&result)
                .map_err(|e| JsValue::from_str(&format!("JSON serialization error: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Subscribe to Welcome messages (persistent subscription for passive mode)
/// Callback receives JSON: { group_id, group_name, kp_event_id }
#[wasm_bindgen]
pub fn subscribe_to_welcome_messages(callback: js_sys::Function) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("📡 Subscribing to Welcome messages (Kind 444) addressed to us...");

            // Get our keys to filter Welcomes addressed to us
            let keys = get_keys()?;
            let pubkey = keys.public_key();

            let client = Arc::new(create_connected_client().await?);

            // Subscribe to Welcomes (Kind 444) with #p tag filtering (addressed to us)
            // No 'since' filter - get all historical Welcomes addressed to us
            let filter = nostr::Filter::new()
                .kind(Kind::Custom(444))
                .pubkey(pubkey); // Filter by #p tag (addressed to us)

            log(&format!("✓ Subscribing to Welcome messages for pubkey: {}", pubkey.to_hex()[..16].to_string()));

            // Spawn background task to listen for notifications with ordered history
            let client_clone = client.clone();
            wasm_bindgen_futures::spawn_local(async move {
                log("📻 Welcome listener started with ordered history");

                let result = subscribe_with_ordered_history(&client_clone, filter, move |welcome_event| {
                    let callback_clone = callback.clone();
                    async move {
                        log(&format!("📩 Processing Welcome event: {}", welcome_event.id.to_hex()));

                        // Extract KeyPackage reference from #e tags
                        let kp_ref: Option<String> = welcome_event.tags.iter()
                            .find_map(|tag| {
                                let tag_vec = tag.clone().to_vec();
                                if tag_vec.get(0).map(|s| s.as_str()) == Some("e") {
                                    tag_vec.get(1).cloned()
                                } else {
                                    None
                                }
                            });

                        if kp_ref.is_none() {
                            log("  No KeyPackage reference found, ignoring");
                            return Ok(());
                        }

                        let kp_event_id = kp_ref.unwrap();
                        log(&format!("  ✅ Welcome references KeyPackage: {}", &kp_event_id[..16.min(kp_event_id.len())]));

                        // Process the Welcome
                        match create_mdk().await {
                            Ok(mdk) => {
                                // Convert to UnsignedEvent
                                let mut rumor = nostr::UnsignedEvent {
                                    id: None,
                                    pubkey: welcome_event.pubkey,
                                    created_at: welcome_event.created_at,
                                    kind: welcome_event.kind,
                                    tags: welcome_event.tags.clone(),
                                    content: welcome_event.content.clone(),
                                };
                                rumor.ensure_id();

                                match mdk.process_welcome(&welcome_event.id, &rumor) {
                                    Ok(welcome) => {
                                        let group_id = hex::encode(welcome.mls_group_id.as_slice());
                                        let group_name = welcome.group_name.clone();
                                        log(&format!("  ✓ Processed Welcome! Group: {}", group_name));

                                        // Check if already accepted (de-duplicate)
                                        use mdk_storage_traits::welcomes::types::WelcomeState;
                                        if welcome.state == WelcomeState::Accepted {
                                            log(&format!("  ℹ️  Welcome already accepted, skipping"));
                                            return Ok(());
                                        }

                                        // Accept Welcome (join the group)
                                        log("  Accepting Welcome (joining group)...");
                                        match mdk.accept_welcome(&welcome) {
                                            Ok(_) => {
                                                log(&format!("✅ Successfully joined group: {}", group_name));

                                                // Explicitly save after accepting Welcome
                                                if let Ok(storage) = get_or_create_storage().await {
                                                    if let Err(e) = storage.inner().save_snapshot() {
                                                        log(&format!("⚠️ Failed to save after accept_welcome: {:?}", e));
                                                    } else {
                                                        log("  ✓ Storage saved");
                                                    }
                                                }

                                                // Call JavaScript callback with result
                                                #[derive(Serialize)]
                                                struct WelcomeResult {
                                                    group_id: String,
                                                    group_name: String,
                                                    kp_event_id: String,
                                                }

                                                let result = WelcomeResult {
                                                    group_id,
                                                    group_name,
                                                    kp_event_id,
                                                };

                                                if let Ok(json) = serde_json::to_string(&result) {
                                                    let js_string = JsValue::from_str(&json);
                                                    let _ = callback_clone.call1(&JsValue::NULL, &js_string);
                                                }
                                            }
                                            Err(e) => {
                                                let error_str = format!("Failed to accept Welcome: {}", e);
                                                log(&format!("❌ {}", error_str));

                                                // Notify JS about the error
                                                #[derive(Serialize)]
                                                struct ErrorResult {
                                                    error: String,
                                                    kp_event_id: String,
                                                }

                                                let result = ErrorResult {
                                                    error: error_str,
                                                    kp_event_id: kp_event_id.clone(),
                                                };

                                                if let Ok(json) = serde_json::to_string(&result) {
                                                    let js_string = JsValue::from_str(&json);
                                                    let _ = callback_clone.call1(&JsValue::NULL, &js_string);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let error_str = format!("{}", e);
                                        log(&format!("❌ Failed to process Welcome: {}", error_str));

                                        // Still notify JS (so it can delete the KeyPackage)
                                        #[derive(Serialize)]
                                        struct ErrorResult {
                                            error: String,
                                            kp_event_id: String,
                                        }

                                        let result = ErrorResult {
                                            error: error_str,
                                            kp_event_id: kp_event_id.clone(),
                                        };

                                        if let Ok(json) = serde_json::to_string(&result) {
                                            let js_string = JsValue::from_str(&json);
                                            let _ = callback_clone.call1(&JsValue::NULL, &js_string);
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                log(&format!("❌ Failed to create MDK: {:?}", e));
                            }
                        }

                        Ok(())
                    }
                }).await;

                if let Err(e) = result {
                    log(&format!("❌ Welcome subscription error: {:?}", e));
                }
            });

            Ok::<(), JsValue>(())
        }
        .await;

        result.map(|_| JsValue::NULL)
    })
}

/// Create a new group and invite members
/// Returns a Promise that resolves to the group ID
#[wasm_bindgen]
pub fn create_group_with_members(name: String, description: String, member_npubs_json: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📝 Creating group: {}", name));

            // Parse member data (array of {npub: string, is_admin: bool})
            #[derive(Deserialize)]
            struct MemberData {
                npub: String,
                is_admin: bool,
            }

            let members: Vec<MemberData> = serde_json::from_str(&member_npubs_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid members JSON: {}", e)))?;

            // Get our keys
            let keys = get_keys()?;
            let our_pubkey = keys.public_key();

            // Fetch KeyPackages for each member
            log(&format!("Fetching KeyPackages for {} member(s)...", members.len()));

            let client = create_connected_client().await?;

            let mut key_package_events = Vec::new();
            let mut admin_pubkeys = vec![our_pubkey]; // Creator is always admin

            for member in &members {
                log(&format!("  Fetching KeyPackage for {}...", &member.npub[..16]));

                // Parse npub to get public key
                let pubkey = nostr::PublicKey::from_bech32(&member.npub)
                    .map_err(|e| JsValue::from_str(&format!("Invalid npub {}: {}", member.npub, e)))?;

                // Add to admin list if flagged as admin
                if member.is_admin {
                    admin_pubkeys.push(pubkey);
                }

                // Query for their most recent KeyPackage (kind 443)
                let filter = nostr::Filter::new()
                    .kind(Kind::Custom(443))
                    .author(pubkey)
                    .limit(10);  // Get last 10, we'll pick the newest non-deleted

                let events = client.fetch_events(filter, Duration::from_secs(10)).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackages for {}: {}", member.npub, e)))?;

                if events.is_empty() {
                    return Err(JsValue::from_str(&format!("No KeyPackage found for {}", member.npub)));
                }

                // Fetch deletion events (Kind 5) from this author to filter out deleted KeyPackages
                let deletion_filter = nostr::Filter::new()
                    .kind(Kind::EventDeletion)
                    .author(pubkey)
                    .limit(50);  // Get recent deletions

                let deletion_events = client.fetch_events(deletion_filter, Duration::from_secs(5)).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to fetch deletions: {}", e)))?;

                // Collect deleted event IDs from 'e' tags
                let deleted_ids: std::collections::HashSet<nostr::EventId> = deletion_events.iter()
                    .flat_map(|del_event| {
                        del_event.tags.iter().filter_map(|tag| {
                            // Extract event ID from 'e' tags
                            let tag_vec = tag.clone().to_vec();
                            tag_vec.get(0)
                                .filter(|&kind| kind == "e")
                                .and_then(|_| tag_vec.get(1))
                                .and_then(|id_str| nostr::EventId::from_hex(id_str).ok())
                        })
                    })
                    .collect();

                log(&format!("    Found {} deletion events covering {} KeyPackages",
                    deletion_events.len(), deleted_ids.len()));

                // Filter out deleted KeyPackages and get the newest remaining one
                let available_kps: Vec<_> = events.iter()
                    .filter(|e| !deleted_ids.contains(&e.id))
                    .collect();

                if available_kps.is_empty() {
                    return Err(JsValue::from_str(&format!("No available (non-deleted) KeyPackage found for {}", member.npub)));
                }

                let newest = available_kps.iter()
                    .max_by_key(|e| e.created_at)
                    .unwrap();

                log(&format!("    ✓ Found available KeyPackage: {} ({} deleted, {} available)",
                    newest.id.to_hex(), deleted_ids.len(), available_kps.len()));
                key_package_events.push((*newest).clone());
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

            // Clone key_package_events so we can use it later to add p tags
            let kp_events_for_tags = key_package_events.clone();

            let group_result = mdk.create_group(&our_pubkey, key_package_events, config)
                .map_err(|e| JsValue::from_str(&format!("Failed to create group: {}", e)))?;

            let group_id = hex::encode(group_result.group.mls_group_id.as_slice());
            log(&format!("✅ Group created! ID: {}", &group_id[..16]));

            // Publish Welcome messages to each invited member
            log(&format!("Publishing {} Welcome message(s)...", group_result.welcome_rumors.len()));

            let mut invitations = Vec::new();

            for mut welcome_unsigned in group_result.welcome_rumors {
                // Extract recipient pubkey from the e tag (KeyPackage reference)
                // The Welcome references a KeyPackage via e tag, and we need to add a p tag for that KeyPackage's author

                // First, extract the kp_event_id as an owned String to avoid borrowing issues
                let kp_event_id_opt = welcome_unsigned.tags.iter()
                    .find(|t| t.kind().as_str() == "e")
                    .and_then(|tag| tag.content().map(|s| s.to_string()));

                if let Some(kp_event_id) = kp_event_id_opt {
                    // Find the KeyPackage event to get its author (the invitee's pubkey)
                    if let Some(kp_event) = kp_events_for_tags.iter().find(|e| e.id.to_hex() == kp_event_id) {
                        let invitee_pubkey = kp_event.pubkey;
                        let invitee_pubkey_hex = invitee_pubkey.to_hex();

                        // Find the corresponding npub
                        let invitee_npub = members.iter()
                            .find(|member| {
                                if let Ok(pk) = nostr::PublicKey::from_bech32(&member.npub) {
                                    pk == invitee_pubkey
                                } else {
                                    false
                                }
                            })
                            .map(|m| m.npub.clone())
                            .unwrap_or_else(|| invitee_pubkey.to_bech32().unwrap());

                        // Add p tag with invitee's pubkey
                        welcome_unsigned.tags.push(nostr::Tag::public_key(invitee_pubkey));
                        log(&format!("  Added p tag for invitee: {}", invitee_pubkey_hex));

                        // Clear the ID so it gets recalculated when signing
                        welcome_unsigned.id = None;

                        // Ensure ID is set before signing
                        welcome_unsigned.ensure_id();

                        // Sign the UnsignedEvent
                        let welcome_event = welcome_unsigned.sign(&keys).await
                            .map_err(|e| JsValue::from_str(&format!("Failed to sign Welcome: {}", e)))?;

                        let welcome_event_id = welcome_event.id.to_hex();

                        let send_result = client.send_event(&welcome_event).await
                            .map_err(|e| JsValue::from_str(&format!("Failed to send Welcome: {}", e)))?;

                        log("Publishing Welcome message:");
                        for relay_url in send_result.success.iter() {
                            log(&format!("  ✓ {} accepted Welcome", relay_url));
                        }
                        for (relay_url, error) in send_result.failed.iter() {
                            log(&format!("  ✗ {} rejected Welcome: {}", relay_url, error));
                        }

                        // Save invitation details
                        invitations.push(serde_json::json!({
                            "invitee_npub": invitee_npub,
                            "invitee_pubkey": invitee_pubkey_hex,
                            "keypackage_event_id": kp_event_id,
                            "welcome_event_id": welcome_event_id,
                            "group_id": group_id.clone(),
                            "group_name": name.clone(),
                            "timestamp": nostr::Timestamp::now().as_u64(),
                        }));
                    }
                }
            }

            log(&format!("✅ All Welcome messages published!"));

            // Explicitly save after creating group (critical operation)
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save after create_group: {:?}", e)))?;
            log("✓ State saved to storage");

            // Disconnect
            let _ = client.disconnect().await;

            // Return group ID and invitations
            let result = serde_json::json!({
                "group_id": group_id,
                "invitations": invitations,
            });

            Ok::<String, JsValue>(result.to_string())
        }
        .await;

        result.map(|group_id| JsValue::from_str(&group_id))
    })
}

/// Create group and publish evolution event
/// Returns welcome_rumors array, member npubs array, and group_id
#[wasm_bindgen]
pub fn create_group_and_publish(name: String, description: String, keypackage_event_ids_json: String, member_npubs_json: String, is_admin_flags_json: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📝 Creating group: {}", name));

            // Parse inputs
            let keypackage_event_ids: Vec<String> = serde_json::from_str(&keypackage_event_ids_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid keypackage IDs JSON: {}", e)))?;
            let member_npubs: Vec<String> = serde_json::from_str(&member_npubs_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid npubs JSON: {}", e)))?;
            let is_admin_flags: Vec<bool> = serde_json::from_str(&is_admin_flags_json)
                .map_err(|e| JsValue::from_str(&format!("Invalid admin flags JSON: {}", e)))?;

            if keypackage_event_ids.len() != member_npubs.len() || member_npubs.len() != is_admin_flags.len() {
                return Err(JsValue::from_str("Mismatched array lengths"));
            }

            // Get our keys
            let keys = get_keys()?;
            let our_pubkey = keys.public_key();

            // Fetch KeyPackage events by ID
            let client = create_connected_client().await?;
            let mut key_package_events = Vec::new();
            let mut admin_pubkeys = vec![our_pubkey]; // Creator is always admin

            for (i, kp_event_id) in keypackage_event_ids.iter().enumerate() {
                log(&format!("Fetching KeyPackage {}...", &kp_event_id[..16]));

                let event_id = nostr::EventId::from_hex(kp_event_id)
                    .map_err(|e| JsValue::from_str(&format!("Invalid event ID: {}", e)))?;

                let filter = nostr::Filter::new().id(event_id);
                let events = client.fetch_events(filter, Duration::from_secs(10)).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackage: {}", e)))?;

                let kp_event = events.into_iter().next()
                    .ok_or_else(|| JsValue::from_str(&format!("KeyPackage not found: {}", kp_event_id)))?;

                // Add to admin list if flagged
                if is_admin_flags[i] {
                    let member_pubkey = nostr::PublicKey::from_bech32(&member_npubs[i])
                        .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;
                    admin_pubkeys.push(member_pubkey);
                }

                key_package_events.push(kp_event);
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
                None, None, None,
                relay_urls,
                admin_pubkeys,
            );

            // Create MDK and create the group
            log("Creating group with MDK...");
            let mdk = create_mdk().await?;
            let group_result = mdk.create_group(&our_pubkey, key_package_events.clone(), config)
                .map_err(|e| JsValue::from_str(&format!("Failed to create group: {}", e)))?;

            let group_id = hex::encode(group_result.group.mls_group_id.as_slice());
            log(&format!("✅ Group created! ID: {}", &group_id[..16]));

            // Save state
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save: {:?}", e)))?;
            log("✓ State saved to storage");

            // Disconnect
            let _ = client.disconnect().await;

            // Convert welcome_rumors to JSON strings
            let welcome_rumors_strings: Vec<String> = group_result.welcome_rumors.iter()
                .map(|rumor| serde_json::to_string(rumor).unwrap())
                .collect();

            // Return result
            let response = serde_json::json!({
                "group_id": group_id,
                "group_name": name,
                "welcome_rumors": welcome_rumors_strings,
                "member_npubs": member_npubs,
                "keypackage_event_ids": keypackage_event_ids,
            });

            Ok::<String, JsValue>(response.to_string())
        }.await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Fetch KeyPackages for a given npub (for progressive UI updates)
#[wasm_bindgen]
pub fn fetch_keypackages_for_npub(member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            let client = create_connected_client().await?;

            let filter = nostr::Filter::new()
                .kind(Kind::Custom(443))
                .author(member_pubkey)
                .limit(10);

            let events = client.fetch_events(filter, Duration::from_secs(10)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackages: {}", e)))?;

            let _ = client.disconnect().await;

            let keypackages: Vec<_> = events.iter().map(|kp| {
                serde_json::json!({
                    "event_id": kp.id.to_hex(),
                    "created_at": kp.created_at.as_u64(),
                })
            }).collect();

            let result = serde_json::json!({
                "total_found": events.len(),
                "keypackages": keypackages,
            });

            Ok::<String, JsValue>(result.to_string())
        }.await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Fetch deletion events for a given npub (for progressive UI updates)
#[wasm_bindgen]
pub fn fetch_deletions_for_npub(member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            let client = create_connected_client().await?;

            let deletion_filter = nostr::Filter::new()
                .kind(Kind::EventDeletion)
                .author(member_pubkey)
                .limit(50);

            let deletion_events = client.fetch_events(deletion_filter, Duration::from_secs(5)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch deletions: {}", e)))?;

            let _ = client.disconnect().await;

            let deletions: Vec<_> = deletion_events.iter().map(|del| {
                let referenced_ids: Vec<String> = del.tags.iter().filter_map(|tag| {
                    let tag_vec = tag.clone().to_vec();
                    tag_vec.get(0)
                        .filter(|&kind| kind == "e")
                        .and_then(|_| tag_vec.get(1))
                        .map(|s| s.clone())
                }).collect();

                serde_json::json!({
                    "event_id": del.id.to_hex(),
                    "created_at": del.created_at.as_u64(),
                    "references": referenced_ids,
                })
            }).collect();

            let result = serde_json::json!({
                "total_found": deletion_events.len(),
                "deletions": deletions,
            });

            Ok::<String, JsValue>(result.to_string())
        }.await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Step 1: Add member to group and publish evolution event
/// Returns welcome_rumors and relay status
#[wasm_bindgen]
pub fn add_member_and_publish(group_id_hex: String, keypackage_event_id: String, member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("👋 Adding {} to group {}...", &member_npub[..16], &group_id_hex[..16]));

            // Parse npub to get public key
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            // Parse group ID
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID: {}", e)))?;
            let group_id = mdk_core::prelude::GroupId::from_slice(&group_id_bytes);

            // Fetch the specific KeyPackage event by ID
            log(&format!("Fetching KeyPackage {}...", &keypackage_event_id[..16]));
            let client = create_connected_client().await?;

            let event_id = nostr::EventId::from_hex(&keypackage_event_id)
                .map_err(|e| JsValue::from_str(&format!("Invalid event ID: {}", e)))?;

            let filter = nostr::Filter::new()
                .id(event_id);

            let events = client.fetch_events(filter, Duration::from_secs(10)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackage: {}", e)))?;

            if events.is_empty() {
                return Err(JsValue::from_str("KeyPackage event not found"));
            }

            let keypackage_event = events.into_iter().next().unwrap();

            // Create MDK
            let mdk = create_mdk().await?;

            // Add member to group (creates MLS commit)
            log("Creating MLS commit to add member...");
            let invite_result = mdk.add_members(&group_id, &[keypackage_event.clone()])
                .map_err(|e| JsValue::from_str(&format!("Failed to add member: {}", e)))?;

            // Merge the commit locally
            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge commit: {}", e)))?;

            // Publish evolution event
            log("Publishing evolution event to relays...");
            let send_result = client.send_event(&invite_result.evolution_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish evolution: {}", e)))?;

            let success_relays: Vec<String> = send_result.success.iter().map(|url| url.to_string()).collect();
            let failed_relays: Vec<_> = send_result.failed.iter()
                .map(|(url, err)| serde_json::json!({"url": url.to_string(), "error": err.to_string()}))
                .collect();

            for relay_url in &success_relays {
                log(&format!("  ✓ {} accepted evolution event", relay_url));
            }
            for failed in &failed_relays {
                log(&format!("  ✗ {} rejected evolution event", failed));
            }

            // Serialize welcome_rumors for return
            let welcome_rumors_json = if let Some(welcome_rumors) = &invite_result.welcome_rumors {
                let rumors: Vec<_> = welcome_rumors.iter().map(|rumor| {
                    serde_json::to_string(rumor).unwrap_or_default()
                }).collect();
                serde_json::to_string(&rumors).unwrap_or_default()
            } else {
                "[]".to_string()
            };

            // Save state
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save: {:?}", e)))?;

            let _ = client.disconnect().await;

            let response = serde_json::json!({
                "member_pubkey": member_pubkey.to_hex(),
                "welcome_rumors": welcome_rumors_json,
                "evolution_relays_success": success_relays,
                "evolution_relays_failed": failed_relays,
            });

            Ok::<String, JsValue>(response.to_string())
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Step 2: Publish Welcome message to relays
#[wasm_bindgen]
pub fn publish_welcome_message(welcome_rumors_json: String, member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("📨 Publishing Welcome message to {}...", &member_npub[..16]));

            // Parse member npub
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            // Parse welcome_rumors from JSON
            let rumors_strings: Vec<String> = serde_json::from_str(&welcome_rumors_json)
                .map_err(|e| JsValue::from_str(&format!("Failed to parse welcome_rumors: {}", e)))?;

            if rumors_strings.is_empty() {
                return Err(JsValue::from_str("No Welcome rumors to publish"));
            }

            // Get keys for signing
            let keys = get_keys()?;

            // Connect to relays
            let client = create_connected_client().await?;

            let mut welcome_event_id = String::new();
            let mut all_success_relays = Vec::new();
            let mut all_failed_relays = Vec::new();

            for rumor_str in rumors_strings {
                let mut welcome_unsigned: nostr::UnsignedEvent = serde_json::from_str(&rumor_str)
                    .map_err(|e| JsValue::from_str(&format!("Failed to parse rumor: {}", e)))?;

                // Add p tag with invitee's pubkey
                welcome_unsigned.tags.push(nostr::Tag::public_key(member_pubkey));

                // Clear and recalculate ID
                welcome_unsigned.id = None;
                welcome_unsigned.ensure_id();

                // Sign the Welcome event
                let welcome_event = welcome_unsigned.sign(&keys).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to sign Welcome: {}", e)))?;

                welcome_event_id = welcome_event.id.to_hex();

                // Publish to relays
                let send_result = client.send_event(&welcome_event).await
                    .map_err(|e| JsValue::from_str(&format!("Failed to send Welcome: {}", e)))?;

                for relay_url in send_result.success.iter() {
                    log(&format!("  ✓ {} accepted Welcome", relay_url));
                    all_success_relays.push(relay_url.to_string());
                }
                for (relay_url, error) in send_result.failed.iter() {
                    log(&format!("  ✗ {} rejected Welcome: {}", relay_url, error));
                    all_failed_relays.push(serde_json::json!({
                        "url": relay_url.to_string(),
                        "error": error.to_string()
                    }));
                }
            }

            let _ = client.disconnect().await;

            log(&format!("✅ Welcome sent to {}!", &member_npub[..16]));

            let response = serde_json::json!({
                "welcome_event_id": welcome_event_id,
                "relays_success": all_success_relays,
                "relays_failed": all_failed_relays,
            });

            Ok::<String, JsValue>(response.to_string())
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

/// Step 3: Promote member to admin and publish
#[wasm_bindgen]
pub fn promote_to_admin_and_publish(group_id_hex: String, member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("👑 Promoting {} to admin...", &member_npub[..16]));

            // Parse member npub
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            // Parse group ID
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID: {}", e)))?;
            let group_id = mdk_core::prelude::GroupId::from_slice(&group_id_bytes);

            // Create MDK
            let mdk = create_mdk().await?;

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

            // Merge the update commit
            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge admin update: {}", e)))?;

            // Publish the admin update evolution event
            let client = create_connected_client().await?;
            let send_result = client.send_event(&update_result.evolution_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish admin update: {}", e)))?;

            let success_relays: Vec<String> = send_result.success.iter().map(|url| url.to_string()).collect();
            let failed_relays: Vec<_> = send_result.failed.iter()
                .map(|(url, err)| serde_json::json!({"url": url.to_string(), "error": err.to_string()}))
                .collect();

            for relay_url in &success_relays {
                log(&format!("  ✓ {} accepted admin update", relay_url));
            }

            // Save state
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save: {:?}", e)))?;

            let _ = client.disconnect().await;

            log("✅ Member promoted to admin!");

            let response = serde_json::json!({
                "relays_success": success_relays,
                "relays_failed": failed_relays,
            });

            Ok::<String, JsValue>(response.to_string())
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}

pub fn invite_member_to_group(group_id_hex: String, member_npub: String, is_admin: bool) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            let admin_status = if is_admin { " as admin" } else { "" };
            log(&format!("👋 Inviting {} to group {}{}", &member_npub[..16], &group_id_hex[..16], admin_status));

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

            // Fetch deletion events (Kind 5) to filter out deleted KeyPackages
            let deletion_filter = nostr::Filter::new()
                .kind(Kind::EventDeletion)
                .author(member_pubkey)
                .limit(50);

            let deletion_events = client.fetch_events(deletion_filter, Duration::from_secs(5)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch deletions: {}", e)))?;

            // Collect deleted event IDs from 'e' tags
            let deleted_ids: std::collections::HashSet<nostr::EventId> = deletion_events.iter()
                .flat_map(|del_event| {
                    del_event.tags.iter().filter_map(|tag| {
                        // Extract event ID from 'e' tags
                        let tag_vec = tag.clone().to_vec();
                        tag_vec.get(0)
                            .filter(|&kind| kind == "e")
                            .and_then(|_| tag_vec.get(1))
                            .and_then(|id_str| nostr::EventId::from_hex(id_str).ok())
                    })
                })
                .collect();

            // Filter out deleted KeyPackages
            let available_kps: Vec<_> = events.iter()
                .filter(|e| !deleted_ids.contains(&e.id))
                .collect();

            if available_kps.is_empty() {
                return Err(JsValue::from_str(&format!("No available (non-deleted) KeyPackage found for {}. They may need to create a new one.", &member_npub[..16])));
            }

            // Get the newest available KeyPackage
            let newest = available_kps.iter()
                .max_by_key(|e| e.created_at)
                .unwrap();

            log(&format!("  ✓ Found available KeyPackage: {} ({} deleted, {} available)",
                newest.id.to_hex(), deleted_ids.len(), available_kps.len()));

            // Get our keys
            let keys = get_keys()?;

            // Create MDK
            let mdk = create_mdk().await?;

            // Step 1: Add member to group
            log("Adding member to group...");
            let invite_result = mdk.add_members(&group_id, &[(**newest).clone()])
                .map_err(|e| JsValue::from_str(&format!("Failed to add member: {}", e)))?;

            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge commit: {}", e)))?;

            client.send_event(&invite_result.evolution_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish evolution: {}", e)))?;

            // Step 3: Publish Welcome message
            let mut welcome_event_id = String::new();
            if let Some(welcome_rumors) = invite_result.welcome_rumors {
                log(&format!("Publishing Welcome message to {}...", &member_npub[..16]));

                for mut welcome_unsigned in welcome_rumors {
                    // Add p tag with invitee's pubkey so they know the Welcome is for them
                    welcome_unsigned.tags.push(nostr::Tag::public_key(member_pubkey));
                    log(&format!("  Added p tag for invitee: {}", member_pubkey.to_hex()));

                    // Clear the ID so it gets recalculated when signing
                    welcome_unsigned.id = None;

                    // Ensure ID is set before signing
                    welcome_unsigned.ensure_id();

                    let welcome_event = welcome_unsigned.sign(&keys).await
                        .map_err(|e| JsValue::from_str(&format!("Failed to sign Welcome: {}", e)))?;

                    // Store the Welcome event ID
                    welcome_event_id = welcome_event.id.to_hex();

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

            // Step 3: Promote new member to admin (if requested)
            if is_admin {
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
            }

            // Explicitly save after inviting member (critical operation)
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save after invite_member: {:?}", e)))?;
            log("✓ State saved to storage");

            log("✅ Invitation complete!");

            // Get group info for return value
            let group_data = mdk.get_group(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get group: {}", e)))?
                .ok_or_else(|| JsValue::from_str("Group not found"))?;

            // Disconnect
            let _ = client.disconnect().await;

            // Return detailed invitation information
            let group_name = if group_data.name.is_empty() {
                "Unnamed Group".to_string()
            } else {
                group_data.name.clone()
            };

            // Build detailed keypackage info
            let keypackages: Vec<_> = events.iter().map(|kp| {
                serde_json::json!({
                    "event_id": kp.id.to_hex(),
                    "created_at": kp.created_at.as_u64(),
                    "deleted": deleted_ids.contains(&kp.id),
                })
            }).collect();

            // Build deletion event info
            let deletions: Vec<_> = deletion_events.iter().map(|del| {
                let referenced_ids: Vec<String> = del.tags.iter().filter_map(|tag| {
                    let tag_vec = tag.clone().to_vec();
                    tag_vec.get(0)
                        .filter(|&kind| kind == "e")
                        .and_then(|_| tag_vec.get(1))
                        .map(|s| s.clone())
                }).collect();

                serde_json::json!({
                    "event_id": del.id.to_hex(),
                    "created_at": del.created_at.as_u64(),
                    "references": referenced_ids,
                })
            }).collect();

            let result = serde_json::json!({
                "success": true,
                "invitee_npub": member_npub,
                "invitee_pubkey": member_pubkey.to_hex(),
                "keypackages": {
                    "total_found": events.len(),
                    "deleted_count": deleted_ids.len(),
                    "available_count": available_kps.len(),
                    "all_keypackages": keypackages,
                    "selected_keypackage_id": newest.id.to_hex(),
                },
                "deletions": deletions,
                "welcome_event_id": welcome_event_id,
                "group_id": group_id_hex,
                "group_name": group_name,
                "timestamp": nostr::Timestamp::now().as_u64(),
            });

            Ok::<JsValue, JsValue>(JsValue::from_str(&result.to_string()))
        }
        .await;

        result
    })
}

/// Remove a member from a group (or leave the group if removing yourself)
/// Returns a Promise that resolves when the removal is complete
#[wasm_bindgen]
pub fn remove_member_from_group(group_id_hex: String, member_npub: String) -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log(&format!("🚪 Removing {} from group {}...", &member_npub[..16], &group_id_hex[..16]));

            // Parse npub to get public key
            let member_pubkey = nostr::PublicKey::from_bech32(&member_npub)
                .map_err(|e| JsValue::from_str(&format!("Invalid npub: {}", e)))?;

            // Parse group ID
            let group_id_bytes = hex::decode(&group_id_hex)
                .map_err(|e| JsValue::from_str(&format!("Invalid group ID: {}", e)))?;
            let group_id = mdk_core::prelude::GroupId::from_slice(&group_id_bytes);

            // Get our keys
            let keys = get_keys()?;
            let our_pubkey = keys.public_key();

            // Create MDK
            let mdk = create_mdk().await?;

            // Get group data to check admins and member
            let group = mdk.get_group(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to get group: {}", e)))?
                .ok_or_else(|| JsValue::from_str("Group not found"))?;

            let is_self_removal = member_pubkey == our_pubkey;

            // If removing someone else, check we're an admin
            if !is_self_removal && !group.admin_pubkeys.contains(&our_pubkey) {
                return Err(JsValue::from_str("Only admins can remove other members"));
            }

            // Check if removing the last admin (only if removing someone else)
            if !is_self_removal && group.admin_pubkeys.contains(&member_pubkey) {
                // Count remaining admins after removal
                let remaining_admins = group.admin_pubkeys.iter()
                    .filter(|&pk| pk != &member_pubkey)
                    .count();

                if remaining_admins == 0 {
                    return Err(JsValue::from_str("Cannot remove the last admin from the group"));
                }
            }

            let client = create_connected_client().await?;

            // Remove the member (use leave_group for self, remove_members for others)
            // Note: We don't send a message beforehand because it would create epoch conflicts
            let remove_result = if is_self_removal {
                log("Leaving group...");
                mdk.leave_group(&group_id)
                    .map_err(|e| JsValue::from_str(&format!("Failed to leave group: {}", e)))?
            } else {
                log("Removing member from group...");
                mdk.remove_members(&group_id, &[member_pubkey])
                    .map_err(|e| JsValue::from_str(&format!("Failed to remove member: {}", e)))?
            };

            // Publish the evolution event FIRST so others can process it
            client.send_event(&remove_result.evolution_event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish removal evolution: {}", e)))?;

            // Then merge our local state
            mdk.merge_pending_commit(&group_id)
                .map_err(|e| JsValue::from_str(&format!("Failed to merge removal commit: {}", e)))?;

            // Explicitly save after removing member
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save after remove_member: {:?}", e)))?;
            log("✓ State saved to storage");

            // Disconnect
            let _ = client.disconnect().await;

            log(&format!("✅ {} removed from group", &member_npub[..16]));

            // Return result as JSON
            #[derive(Serialize)]
            struct RemovalResult {
                success: bool,
                group_id: String,
                removed_member: String,
                is_self_removal: bool,
            }

            let result = RemovalResult {
                success: true,
                group_id: group_id_hex,
                removed_member: member_npub,
                is_self_removal,
            };

            let json = serde_json::to_string(&result)
                .map_err(|e| JsValue::from_str(&format!("JSON serialization error: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
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

            // Explicitly save after accepting all Welcomes (critical operation)
            if accepted > 0 {
                let storage = get_or_create_storage().await?;
                storage.inner().save_snapshot()
                    .map_err(|e| JsValue::from_str(&format!("Failed to save after accept_welcome: {:?}", e)))?;
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
                .map_err(|e| {
                    use mdk_core::error::Error;
                    if matches!(e, Error::OwnLeafNotFound) {
                        JsValue::from_str("You have been removed from this group and can no longer send messages")
                    } else {
                        JsValue::from_str(&format!("Failed to create message: {}", e))
                    }
                })?;
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

            // Explicitly save after sending message (critical operation)
            let storage = get_or_create_storage().await?;
            storage.inner().save_snapshot()
                .map_err(|e| JsValue::from_str(&format!("Failed to save after send_message: {:?}", e)))?;
            log("  ✓ State saved to storage");

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

            // Get cached storage (don't create new instance!)
            let storage = get_or_create_storage().await?;

            // Get messages using the GroupStorage trait
            use mdk_storage_traits::groups::GroupStorage;
            let messages = storage.inner().messages(&group_id)
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
                    pubkey: msg.pubkey.to_bech32().unwrap_or_else(|_| msg.pubkey.to_hex()),
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
                                                    pubkey: msg.pubkey.to_bech32().unwrap_or_else(|_| msg.pubkey.to_hex()),
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
                                        } else if matches!(e, Error::OwnLeafNotFound) {
                                            log(&format!("  ℹ️  You have been removed from this group"));

                                            // Show user-friendly notification
                                            if let Some(window) = web_sys::window() {
                                                let _ = window.alert_with_message(
                                                    "🚪 You've been removed from this group\n\n\
                                                    An admin has removed you from the group.\n\
                                                    You can no longer send or receive messages."
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

