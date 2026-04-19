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

pub mod battery;
pub mod bios;
pub mod collector;
pub mod cpu;
pub mod disk;
pub mod error;
pub mod gpu;
pub mod memory;
pub mod models;
pub mod network;
pub mod os;
pub mod platform;
pub mod process;
pub mod software;

#[cfg(feature = "axum")]
pub mod handlers;

pub use collector::{CollectorConfig, collect_inventory, collect_quick_inventory};
pub use error::InventoryError;
pub use models::SystemInventory;
