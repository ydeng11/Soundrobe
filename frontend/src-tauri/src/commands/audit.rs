//! Audit runner and cancellation commands.

use crate::state::audit::AuditState;
use tauri::State;

#[tauri::command]
pub fn audit_cancel(state: State<'_, AuditState>) {
    state.cancel();
}
