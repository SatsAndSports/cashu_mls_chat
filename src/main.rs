use anyhow::Result;

// MDK imports
use mdk_core::MDK;
use mdk_memory_storage::MdkMemoryStorage;

// CDK imports
use cdk::Amount;

#[tokio::main]
async fn main() -> Result<()> {
    println!("Testing MDK integration...");
    let _mdk = MDK::new(MdkMemoryStorage::default());
    println!("✓ MDK initialized successfully");

    println!("\nTesting CDK integration...");
    let amount = Amount::from(100);
    println!("✓ CDK Amount created: {} sats", amount);

    println!("\nBoth libraries integrated successfully!");
    Ok(())
}
