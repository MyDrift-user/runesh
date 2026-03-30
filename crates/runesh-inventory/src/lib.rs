//! Cross-platform hardware and software inventory collection.
//!
//! Provides comprehensive system inventory including CPU, memory, disk, GPU,
//! network interfaces, BIOS/motherboard, battery, installed software, and
//! running processes. Works on Windows, macOS, and Linux.
//!
//! # Quick Start
//!
//! ```ignore
//! use runesh_inventory::{collect_inventory, CollectorConfig};
//!
//! let config = CollectorConfig::default();
//! let inventory = collect_inventory(&config)?;
//! println!("CPU: {}", inventory.cpu.brand);
//! println!("RAM: {} GB", inventory.memory.total_bytes / 1_073_741_824);
//! ```
//!
//! # Axum Integration
//!
//! ```ignore
//! use axum::{Router, routing::get};
//! use runesh_inventory::handlers;
//!
//! let app = Router::new()
//!     .route("/api/inventory", get(handlers::get_full_inventory))
//!     .route("/api/inventory/quick", get(handlers::get_quick_inventory));
//! ```

pub mod error;
pub mod models;
pub mod collector;
pub mod cpu;
pub mod memory;
pub mod disk;
pub mod network;
pub mod gpu;
pub mod os;
pub mod bios;
pub mod battery;
pub mod software;
pub mod process;
pub mod platform;

#[cfg(feature = "axum")]
pub mod handlers;

pub use collector::{collect_inventory, collect_quick_inventory, CollectorConfig};
pub use error::InventoryError;
pub use models::SystemInventory;
