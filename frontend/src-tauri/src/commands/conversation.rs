//! Assistant runtime initialization and conversation-history query commands.

use crate::error::ApiError;
use crate::state::assistant::AssistantRuntimeState;
use crate::state::config::ConfigState;
use crate::state::conversation::{
    ConversationEntry, ConversationState, CurrentSession, SessionSummary,
};
use tauri::State;

#[tauri::command]
pub fn assistant_init_runtime(
    conversation: State<'_, ConversationState>,
    runtime: State<'_, AssistantRuntimeState>,
    config: State<'_, ConfigState>,
) -> Result<(), ApiError> {
    let cache_path = config.raw().cache_path;
    if !conversation.initialize(cache_path.as_deref()) {
        return Err(ApiError::Message(
            "Failed to initialize assistant session storage".to_string(),
        ));
    }
    runtime.initialize().then_some(()).ok_or_else(|| {
        ApiError::Message("Failed to initialize assistant runtime state".to_string())
    })
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
