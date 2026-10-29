//! Port of `main/proxy/services/hyperspace/**`.

pub mod route;

pub use route::{
    adjust_reasoning_effort_for_tools, call_hyperspace, call_hyperspace_responses,
    fetch_hyperspace_models, install_model_mapping_hook, is_hyperspace_model,
    load_hyperspace_config, register_hyperspace_models, registered_models,
    resolve_hyperspace_model, HyperspaceConfig,
};