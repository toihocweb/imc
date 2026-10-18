//! Port of `main/proxy/lib/imc_memory.py` (the proxy-side memory bridge).

pub mod lib;

pub use lib::{
    append_learned_memories_to_payload, apply_system_reminder_template, detect_project,
    estimate_tokens, format_memories, get_conversation_user_query, get_last_user_text,
    get_project_learned_memories, get_session_project, get_system_text, inject_memories_into_system,
    log_interaction_for_project, rank_candidates, select_memories, set_session_project, tokens,
};