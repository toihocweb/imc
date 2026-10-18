//! Port of `main/proxy/lib/**` (types, config, helpers).
//! Contract: Rust struct = Python class name (PascalCase), fields snake_case
//! mirroring Python attributes, fn names snake_case identical to Python — so the
//! sibling imc-proxy-copilot / imc-proxy crates can translate code 1:1.
//! One Rust module per Python file.

pub mod activity_broker;
pub mod activity_extract;
pub mod api_config;
pub mod atomic_file;
pub mod config_store;
pub mod error;
pub mod logger;
pub mod model_mapping;
pub mod models;
pub mod paths;
pub mod process_cleanup;
pub mod provider_model;
pub mod request_context;
pub mod state;
pub mod types;
pub mod usage_logger;
pub mod utils;

// Re-export the pieces sibling crates reach for most.
pub use config_store::{get_config, AppConfig};
pub use error::{forward_error, forward_generic_error, HTTPError, JsonResponse};
pub use logger::create_handler_logger;
pub use state::{state, State};
