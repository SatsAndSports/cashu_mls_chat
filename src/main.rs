use anyhow::Result;
use eframe::egui;
use std::sync::{Arc, Mutex};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

// MDK imports
use mdk_core::prelude::*;
use mdk_memory_storage::MdkMemoryStorage;
use nostr::event::builder::EventBuilder;
use nostr::{EventId, Keys, Kind, RelayUrl};
use nostr_sdk::{Client, Filter, RelayPoolNotification};

// CDK imports
use cdk::Amount;

#[derive(Clone)]
struct User {
    name: String,
    keys: Keys,
    mdk: Arc<Mutex<MDK<MdkMemoryStorage>>>,
    wallet_balance: Amount,
    mls_group_id: Option<GroupId>,
    nostr_client: Client,
}

#[derive(Clone)]
struct Message {
    sender: String,
    content: String,
}

#[derive(Clone)]
struct AppState {
    users: Vec<User>,
    messages: Arc<Mutex<Vec<Message>>>,
    relay_urls: Vec<RelayUrl>,
    relay_mode: bool, // Set at startup, immutable
}

impl AppState {
    async fn new(relay_mode: bool) -> Result<Self> {
        // Use multiple real relays
        let relay_urls = vec![
            RelayUrl::parse("wss://relay.damus.io")?,
            RelayUrl::parse("wss://nos.lol")?,
            RelayUrl::parse("wss://relay.nostr.band")?,
            RelayUrl::parse("wss://relay.primal.net")?,
            RelayUrl::parse("wss://nostr.bitcoiner.social")?,
            RelayUrl::parse("wss://nostr.mom")?,
            RelayUrl::parse("wss://nostr.oxtr.dev")?,
        ];

        if relay_mode {
            tracing::info!("Starting in RELAY MODE - using real Nostr relays");
        } else {
            tracing::info!("Starting in LOCAL MODE - simulating relay broadcast");
        }

        // Create three users
        let alice_keys = Keys::generate();
        let alice_mdk = Arc::new(Mutex::new(MDK::new(MdkMemoryStorage::default())));
        let alice_client = Client::new(alice_keys.clone());

        let bob_keys = Keys::generate();
        let bob_mdk = Arc::new(Mutex::new(MDK::new(MdkMemoryStorage::default())));
        let bob_client = Client::new(bob_keys.clone());

        let carol_keys = Keys::generate();
        let carol_mdk = Arc::new(Mutex::new(MDK::new(MdkMemoryStorage::default())));
        let carol_client = Client::new(carol_keys.clone());

        // Add relays to clients (for when real relays are enabled)
        for relay_url in &relay_urls {
            alice_client.add_relay(relay_url.as_str()).await?;
            bob_client.add_relay(relay_url.as_str()).await?;
            carol_client.add_relay(relay_url.as_str()).await?;
        }

        // Create key packages for Bob and Carol
        let (bob_key_package, bob_tags) = bob_mdk
            .lock()
            .unwrap()
            .create_key_package_for_event(&bob_keys.public_key(), relay_urls.clone())?;

        let bob_key_package_event = EventBuilder::new(Kind::MlsKeyPackage, bob_key_package)
            .tags(bob_tags)
            .build(bob_keys.public_key())
            .sign(&bob_keys)
            .await?;

        let (carol_key_package, carol_tags) = carol_mdk
            .lock()
            .unwrap()
            .create_key_package_for_event(&carol_keys.public_key(), relay_urls.clone())?;

        let carol_key_package_event = EventBuilder::new(Kind::MlsKeyPackage, carol_key_package)
            .tags(carol_tags)
            .build(carol_keys.public_key())
            .sign(&carol_keys)
            .await?;

        // Alice creates a group with Bob and Carol
        let config = NostrGroupConfigData::new(
            "Group Chat".to_string(),
            "Demo group chat".to_string(),
            None,
            None,
            None,
            relay_urls.clone(),
            vec![
                alice_keys.public_key(),
                bob_keys.public_key(),
                carol_keys.public_key(),
            ],
        );

        let group_result = alice_mdk.lock().unwrap().create_group(
            &alice_keys.public_key(),
            vec![bob_key_package_event, carol_key_package_event],
            config,
        )?;

        let alice_group_id = group_result.group.mls_group_id.clone();

        // Log the group ID for debugging
        let group_id_hex = hex::encode(alice_group_id.as_slice());
        tracing::info!("MLS Group created with ID: {}", group_id_hex);

        // Bob processes welcome and joins
        let bob_welcome_rumor = &group_result.welcome_rumors[0];
        bob_mdk
            .lock()
            .unwrap()
            .process_welcome(&EventId::all_zeros(), bob_welcome_rumor)?;
        let bob_welcomes = bob_mdk.lock().unwrap().get_pending_welcomes()?;
        bob_mdk.lock().unwrap().accept_welcome(&bob_welcomes[0])?;
        let bob_group_id = bob_mdk.lock().unwrap().get_groups()?[0]
            .mls_group_id
            .clone();

        // Carol processes welcome and joins
        let carol_welcome_rumor = &group_result.welcome_rumors[1];
        carol_mdk
            .lock()
            .unwrap()
            .process_welcome(&EventId::all_zeros(), carol_welcome_rumor)?;
        let carol_welcomes = carol_mdk.lock().unwrap().get_pending_welcomes()?;
        carol_mdk
            .lock()
            .unwrap()
            .accept_welcome(&carol_welcomes[0])?;
        let carol_group_id = carol_mdk.lock().unwrap().get_groups()?[0]
            .mls_group_id
            .clone();

        let users = vec![
            User {
                name: "Alice".to_string(),
                keys: alice_keys,
                mdk: alice_mdk,
                wallet_balance: Amount::from(1000),
                mls_group_id: Some(alice_group_id),
                nostr_client: alice_client,
            },
            User {
                name: "Bob".to_string(),
                keys: bob_keys,
                mdk: bob_mdk,
                wallet_balance: Amount::from(500),
                mls_group_id: Some(bob_group_id),
                nostr_client: bob_client,
            },
            User {
                name: "Carol".to_string(),
                keys: carol_keys,
                mdk: carol_mdk,
                wallet_balance: Amount::from(750),
                mls_group_id: Some(carol_group_id),
                nostr_client: carol_client,
            },
        ];

        let state = Self {
            users,
            messages: Arc::new(Mutex::new(Vec::new())),
            relay_urls,
            relay_mode,
        };

        // If relay mode, connect to relays and start listening
        if relay_mode {
            for user in &state.users {
                user.nostr_client.connect().await;
            }
            tracing::info!("Connected to Nostr relays");

            // Start background tasks to listen for messages
            state.start_relay_listeners().await?;
        }

        Ok(state)
    }

    async fn start_relay_listeners(&self) -> Result<()> {
        for (user_index, user) in self.users.iter().enumerate() {
            let client = user.nostr_client.clone();
            let messages = self.messages.clone();
            let user_name = user.name.clone();
            let mdk = user.mdk.clone();
            let group_id = user.mls_group_id.clone().unwrap();

            // Convert group ID to hex string for filtering
            let group_id_hex = hex::encode(group_id.as_slice());

            tokio::spawn(async move {
                tracing::info!("{} starting relay listener", user_name);

                // Subscribe to group messages for OUR specific group only
                let filter = Filter::new()
                    .kind(nostr::Kind::MlsGroupMessage)
                    .custom_tag(
                        nostr::SingleLetterTag::lowercase(nostr::Alphabet::H),
                        group_id_hex.clone()
                    );

                tracing::info!("{} subscribing with filter: kind={:?}, h={}",
                    user_name, nostr::Kind::MlsGroupMessage, group_id_hex);

                match client.subscribe(filter.clone(), None).await {
                    Ok(sub_id) => {
                        tracing::info!("{} subscription successful! ID: {:?}", user_name, sub_id);
                    }
                    Err(e) => {
                        tracing::error!("{} subscription FAILED: {}", user_name, e);
                        return;
                    }
                }

                // Listen for notifications
                tracing::info!("{} starting notification loop...", user_name);
                let mut notifications = client.notifications();
                let mut event_count = 0;

                while let Ok(notification) = notifications.recv().await {
                    match &notification {
                        RelayPoolNotification::Event { relay_url, event, .. } => {
                            event_count += 1;
                            tracing::info!("{} received event #{} from {} (kind: {})", user_name, event_count, relay_url, event.kind);

                            // Try to process the message through MDK
                            match mdk.lock().unwrap().process_message(event) {
                            Ok(_) => {
                                // Get the decrypted messages
                                if let Ok(msgs) = mdk.lock().unwrap().get_messages(&group_id) {
                                    if let Some(last_msg) = msgs.last() {
                                        // Use pubkey prefix as sender identifier
                                        let sender_name = format!("User-{}", &last_msg.pubkey.to_string()[..8]);

                                        messages.lock().unwrap().push(Message {
                                            sender: sender_name,
                                            content: last_msg.content.clone(),
                                        });
                                        tracing::info!("{} received message from relay", user_name);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::debug!("{} couldn't process event: {}", user_name, e);
                            }
                        }
                        }
                        other => {
                            tracing::debug!("{} received other notification: {:?}", user_name, other);
                        }
                    }
                }

                tracing::warn!("{} notification loop ended!", user_name);
            });
        }
        Ok(())
    }

    async fn send_message(&self, user_index: usize, content: String) -> Result<()> {
        let user = &self.users[user_index];
        let group_id = user.mls_group_id.as_ref().unwrap();

        // Create message
        let rumor = EventBuilder::new(Kind::Custom(9), &content).build(user.keys.public_key());

        let message_event = user
            .mdk
            .lock()
            .unwrap()
            .create_message(group_id, rumor)?;

        if self.relay_mode {
            // Log event details before publishing
            tracing::info!("{} sending message event:", user.name);
            tracing::info!("  Event ID: {}", message_event.id);
            tracing::info!("  Kind: {}", message_event.kind);
            tracing::info!("  Tags (first 3): {:?}", message_event.tags.iter().take(3).collect::<Vec<_>>());

            // Publish to real Nostr relays
            let send_result = user.nostr_client.send_event(&message_event).await?;
            tracing::info!("{} published message to Nostr relays", user.name);

            // Log which relays accepted the event
            for relay_url in send_result.success.iter() {
                tracing::info!("  ✓ {} accepted the message", relay_url);
            }
            for relay_url in send_result.failed.keys() {
                if let Some(error) = send_result.failed.get(relay_url) {
                    tracing::warn!("  ✗ {} rejected the message: {}", relay_url, error);
                }
            }

            // Don't add to message list - will come back via subscription
        } else {
            // Simulate local broadcast (no network)
            for other_user in &self.users {
                other_user
                    .mdk
                    .lock()
                    .unwrap()
                    .process_message(&message_event)?;
            }

            // Add to message list in local mode only
            self.messages.lock().unwrap().push(Message {
                sender: user.name.clone(),
                content,
            });
        }

        Ok(())
    }
}

struct ChatApp {
    state: AppState,
    input_texts: [String; 3],
    zoom_level: f32,
}

impl ChatApp {
    fn new(state: AppState) -> Self {
        Self {
            state,
            input_texts: [String::new(), String::new(), String::new()],
            zoom_level: 2.0,
        }
    }

    fn render_user_pane(&mut self, ui: &mut egui::Ui, user_index: usize) {
        let user = &self.state.users[user_index];

        ui.vertical(|ui| {
            ui.heading(&user.name);
            ui.separator();

            // Wallet balance
            ui.horizontal(|ui| {
                ui.label("Balance:");
                ui.label(format!("{} sats", user.wallet_balance));
            });
            ui.separator();

            // Messages
            ui.label("Messages:");
            egui::ScrollArea::vertical()
                .id_salt(format!("messages_{}", user_index))
                .max_height(300.0)
                .show(ui, |ui| {
                    let messages = self.state.messages.lock().unwrap();
                    for msg in messages.iter() {
                        ui.label(format!("{}: {}", msg.sender, msg.content));
                    }
                });

            ui.separator();

            // Input area
            ui.label("Send message:");
            let response = ui.text_edit_singleline(&mut self.input_texts[user_index]);
            let should_send = (ui.button("Send").clicked() || response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                && !self.input_texts[user_index].is_empty();

            if should_send {
                let content = self.input_texts[user_index].clone();
                self.input_texts[user_index].clear();

                let state = self.state.clone();

                // Send message in background
                tokio::spawn(async move {
                    if let Err(e) = state.send_message(user_index, content).await {
                        eprintln!("Error sending message: {}", e);
                    }
                });
            }
        });
    }
}

impl eframe::App for ChatApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply zoom
        ctx.set_pixels_per_point(self.zoom_level);

        egui::TopBottomPanel::top("zoom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Zoom:");
                if ui.button("➖").clicked() && self.zoom_level > 0.5 {
                    self.zoom_level -= 0.1;
                }
                ui.label(format!("{:.0}%", self.zoom_level * 100.0));
                if ui.button("➕").clicked() && self.zoom_level < 3.0 {
                    self.zoom_level += 0.1;
                }
                if ui.button("Reset").clicked() {
                    self.zoom_level = 1.0;
                }

                ui.separator();

                // Show relay mode (read-only, set at startup)
                ui.label(if self.state.relay_mode {
                    "Mode: Real Nostr Relays ✓"
                } else {
                    "Mode: Local Simulation"
                });
                if self.state.relay_mode {
                    ui.label("(7 relays: damus, nos.lol, nostr.band, primal, bitcoiner.social, nostr.mom, oxtr.dev)");
                } else {
                    ui.label("(restart with --relay flag for real relays)");
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(3, |columns| {
                self.render_user_pane(&mut columns[0], 0); // Alice
                self.render_user_pane(&mut columns[1], 1); // Bob
                self.render_user_pane(&mut columns[2], 2); // Carol
            });
        });

        // Request repaint to update messages
        ctx.request_repaint();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    // Check for --relay flag
    let args: Vec<String> = std::env::args().collect();
    let relay_mode = args.contains(&"--relay".to_string());

    // Initialize app state
    println!("Initializing group chat...");
    let state = AppState::new(relay_mode).await?;
    println!("Group chat initialized!");

    // Create single window with three panes
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MDK Group Chat",
        options,
        Box::new(|_cc| Ok(Box::new(ChatApp::new(state)))),
    )
    .map_err(|e| anyhow::anyhow!("Failed to run window: {}", e))?;

    Ok(())
}
