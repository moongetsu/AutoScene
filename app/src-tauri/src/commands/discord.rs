use tauri::State;

use crate::discord::{DiscordActivityPayload, DiscordRpcManager};
use crate::error::AppResult;

#[tauri::command]
#[specta::specta]
pub fn discord_rpc_start(client_id: Option<String>, manager: State<'_, DiscordRpcManager>) -> AppResult<()> {
    manager.start(client_id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn discord_rpc_update(
    activity: Option<DiscordActivityPayload>,
    manager: State<'_, DiscordRpcManager>,
) -> AppResult<()> {
    manager.update(activity);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn discord_rpc_stop(manager: State<'_, DiscordRpcManager>) -> AppResult<()> {
    manager.stop();
    Ok(())
}
