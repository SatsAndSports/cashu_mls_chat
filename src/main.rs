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

// CDK imports
use cdk::Amount;

#[derive(Clone)]
struct User {
    name: String,
    keys: Keys,
    mdk: Arc<Mutex<MDK<MdkMemoryStorage>>>,
    wallet_balance: Amount,
    mls_group_id: Option<GroupId>,
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
    relay_url: RelayUrl,
}

impl AppState {
    async fn new() -> Result<Self> {
        let relay_url = RelayUrl::parse("wss://relay.example.com")?;

        // Create three users
        let alice_keys = Keys::generate();
        let alice_mdk = Arc::new(Mutex::new(MDK::new(MdkMemoryStorage::default())));

        let bob_keys = Keys::generate();
        let bob_mdk = Arc::new(Mutex::new(MDK::new(MdkMemoryStorage::default())));

        let carol_keys = Keys::generate();
        let carol_mdk = Arc::new(Mutex::new(MDK::new(MdkMemoryStorage::default())));

        // Create key packages for Bob and Carol
        let (bob_key_package, bob_tags) = bob_mdk
            .lock()
            .unwrap()
            .create_key_package_for_event(&bob_keys.public_key(), [relay_url.clone()])?;

        let bob_key_package_event = EventBuilder::new(Kind::MlsKeyPackage, bob_key_package)
            .tags(bob_tags)
            .build(bob_keys.public_key())
            .sign(&bob_keys)
            .await?;

        let (carol_key_package, carol_tags) = carol_mdk
            .lock()
            .unwrap()
            .create_key_package_for_event(&carol_keys.public_key(), [relay_url.clone()])?;

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
            vec![relay_url.clone()],
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
            },
            User {
                name: "Bob".to_string(),
                keys: bob_keys,
                mdk: bob_mdk,
                wallet_balance: Amount::from(500),
                mls_group_id: Some(bob_group_id),
            },
            User {
                name: "Carol".to_string(),
                keys: carol_keys,
                mdk: carol_mdk,
                wallet_balance: Amount::from(750),
                mls_group_id: Some(carol_group_id),
            },
        ];

        Ok(Self {
            users,
            messages: Arc::new(Mutex::new(Vec::new())),
            relay_url,
        })
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

        // Process message for all users (simulating Nostr relay broadcast)
        for other_user in &self.users {
            other_user
                .mdk
                .lock()
                .unwrap()
                .process_message(&message_event)?;
        }

        // Add to message list
        self.messages.lock().unwrap().push(Message {
            sender: user.name.clone(),
            content,
        });

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

    // Initialize app state
    println!("Initializing group chat...");
    let state = AppState::new().await?;
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
