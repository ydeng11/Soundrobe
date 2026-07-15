//! Assistant runtime initialization and conversation-history query commands.

use crate::state::config::ConfigState;
use crate::state::conversation::{
    ConversationEntry, ConversationState, CurrentSession, SessionSummary,
};
use tauri::State;

#[tauri::command]
pub fn assistant_init_runtime(
    conversation: State<'_, ConversationState>,
    config: State<'_, ConfigState>,
) {
    let cache_path = config.raw().cache_path;
    let _ = conversation.initialize(cache_path.as_deref());
}

#[tauri::command]
pub fn assistant_list_sessions(
    limit: Option<i64>,
    conversation: State<'_, ConversationState>,
) -> Vec<SessionSummary> {
    conversation.sessions(limit.unwrap_or(50))
}

#[tauri::command]
pub fn assistant_get_conversation(
    session_uuid_or_number: String,
    conversation: State<'_, ConversationState>,
) -> Vec<ConversationEntry> {
    conversation.conversation(&session_uuid_or_number)
}

#[tauri::command]
pub fn assistant_get_session(
    session_uuid_or_number: String,
    conversation: State<'_, ConversationState>,
) -> Option<SessionSummary> {
    conversation.session(&session_uuid_or_number)
}

#[tauri::command]
pub fn assistant_current_session(
    conversation: State<'_, ConversationState>,
) -> Option<CurrentSession> {
    conversation.current()
}
