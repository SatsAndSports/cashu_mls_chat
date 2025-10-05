use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use nostr::{Keys, ToBech32, EventBuilder, Kind, RelayUrl};
use nostr_sdk::Client;
use web_sys::{window, Storage};
use std::sync::Arc;
use std::str::FromStr;
use std::time::Duration;

mod wallet_db;
use wallet_db::HybridWalletDatabase;

mod mdk_storage;
use mdk_storage::MdkHybridStorage;

use cdk::wallet::{Wallet, WalletBuilder, ReceiveOptions};
use cdk::nuts::{CurrencyUnit, Token};
use cdk::mint_url::MintUrl;

use mdk_core::MDK;

// Relay URLs
const RELAYS: &[&str] = &[
    "ws://localhost:8080",
    "wss://nostr.chaima.info",
    "wss://orangesync.tech",
];

/// Helper function to create MDK instance
async fn create_mdk() -> Result<MDK<MdkHybridStorage>, JsValue> {
    let storage = MdkHybridStorage::new().await?;
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

/// Create and broadcast a new KeyPackage to Nostr relays
/// Returns a Promise that resolves to the event ID
#[wasm_bindgen]
pub fn create_and_broadcast_key_package() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Creating KeyPackage...");

            // Get keys and create MDK
            let keys = get_keys()?;
            let mdk = create_mdk().await?;

            // Create KeyPackage for event
            let relay_urls: Vec<RelayUrl> = RELAYS
                .iter()
                .filter_map(|r| RelayUrl::parse(r).ok())
                .collect();

            let (key_package_hex, tags) = mdk
                .create_key_package_for_event(&keys.public_key(), relay_urls)
                .map_err(|e| JsValue::from_str(&format!("Failed to create KeyPackage: {}", e)))?;

            log(&format!("KeyPackage created: {}...", &key_package_hex[..20.min(key_package_hex.len())]));

            // Build and sign event (kind 443)
            let event = EventBuilder::new(Kind::Custom(443), key_package_hex)
                .tags(tags.to_vec())
                .sign_with_keys(&keys)
                .map_err(|e| JsValue::from_str(&format!("Failed to sign event: {}", e)))?;

            let event_id = event.id.to_hex();
            log(&format!("Event signed: {}", event_id));

            // Create client and publish to relays
            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;

            log("Publishing to relays...");
            client.send_event(&event).await
                .map_err(|e| JsValue::from_str(&format!("Failed to publish: {}", e)))?;

            log(&format!("✅ KeyPackage published! Event ID: {}", event_id));

            Ok::<String, JsValue>(event_id)
        }
        .await;

        result.map(|event_id| JsValue::from_str(&event_id))
    })
}

/// Get all groups from MDK storage
/// Returns a Promise that resolves to a JSON array of groups
#[wasm_bindgen]
pub fn get_groups() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching groups from MDK...");

            let mdk = create_mdk().await?;
            let groups = mdk
                .get_groups()
                .map_err(|e| JsValue::from_str(&format!("Failed to get groups: {}", e)))?;

            log(&format!("Found {} group(s)", groups.len()));

            // Convert to JSON array
            let groups_json: Vec<_> = groups.iter().map(|g| {
                serde_json::json!({
                    "id": hex::encode(g.mls_group_id.as_slice()),
                    "name": g.name,
                    "description": g.description,
                    "image_hash": g.image_hash.map(|h| hex::encode(h)),
                    "last_message_at": g.last_message_at.map(|t| t.as_u64()),
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
            let client = Client::default();
            for relay in RELAYS {
                if let Ok(url) = RelayUrl::parse(relay) {
                    let _ = client.add_relay(url).await;
                }
            }
            client.connect().await;

            // Step 1: Get our KeyPackage event IDs
            log("Finding our KeyPackage events...");
            let kp_filter = nostr::Filter::new()
                .kind(Kind::Custom(443))
                .author(pubkey);

            let kp_events = client.fetch_events(kp_filter, Duration::from_secs(5)).await
                .map_err(|e| JsValue::from_str(&format!("Failed to fetch KeyPackages: {}", e)))?;

            let kp_event_ids: Vec<String> = kp_events.iter().map(|e| e.id.to_hex()).collect();
            log(&format!("Found {} KeyPackage(s): {:?}", kp_event_ids.len(), kp_event_ids));

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

/// Fetch KeyPackages from Nostr relays
/// Returns a Promise that resolves to a JSON array of KeyPackage info with relay sources
#[wasm_bindgen]
pub fn fetch_my_key_packages() -> js_sys::Promise {
    future_to_promise(async move {
        let result = async {
            log("Fetching KeyPackages from Nostr...");

            // Get our public key
            let keys = get_keys()?;
            let pubkey = keys.public_key();

            // Build filter for kind 443 events authored by us
            let filter = nostr::Filter::new()
                .kind(Kind::Custom(443))
                .author(pubkey);

            // Query each relay individually and track sources
            use std::collections::HashMap;
            let mut event_relays: HashMap<String, Vec<String>> = HashMap::new();
            let mut all_events: HashMap<String, nostr::Event> = HashMap::new();

            for relay_url in RELAYS {
                log(&format!("Querying {}...", relay_url));

                let client = Client::default();
                if let Ok(url) = RelayUrl::parse(relay_url) {
                    let _ = client.add_relay(url).await;
                    client.connect().await;

                    match client.fetch_events(filter.clone(), Duration::from_secs(5)).await {
                        Ok(events) => {
                            log(&format!("  Found {} event(s) on {}", events.len(), relay_url));
                            for event in events {
                                let event_id = event.id.to_hex();

                                // Track which relay has this event
                                event_relays.entry(event_id.clone())
                                    .or_insert_with(Vec::new)
                                    .push(relay_url.to_string());

                                // Store event (only once)
                                all_events.entry(event_id)
                                    .or_insert(event);
                            }
                        }
                        Err(e) => {
                            log(&format!("  Error querying {}: {}", relay_url, e));
                        }
                    }

                    // Disconnect from this relay
                    let _ = client.disconnect().await;
                }
            }

            log(&format!("Found {} unique KeyPackage(s)", all_events.len()));

            // Convert to JSON array with relay info
            let packages: Vec<_> = all_events.iter().map(|(event_id, event)| {
                let relays = event_relays.get(event_id).cloned().unwrap_or_default();
                serde_json::json!({
                    "event_id": event.id.to_hex(),
                    "created_at": event.created_at.as_u64(),
                    "content_preview": &event.content[..20.min(event.content.len())],
                    "relays": relays,
                })
            }).collect();

            let json = serde_json::to_string(&packages)
                .map_err(|e| JsValue::from_str(&format!("Failed to serialize: {}", e)))?;

            Ok::<String, JsValue>(json)
        }
        .await;

        result.map(|json| JsValue::from_str(&json))
    })
}
