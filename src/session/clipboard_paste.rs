use colored::Colorize;

use super::Session;

/// If no images were explicitly staged (e.g. via Ctrl+V), check the OS
/// clipboard now. This catches image paste via Cmd+V / Ctrl+Shift+V
/// where the terminal may have consumed the paste event without
/// forwarding it to the app (e.g. iTerm2 rendering inline).
///
/// Deduped by content hash against the last auto-attached image — without
/// this, a stale screenshot from turns ago would silently resend itself on
/// every subsequent turn (token waste, and a quiet privacy leak). Explicit
/// `/paste`, `/attach`, and Ctrl+V bypass this check entirely.
pub(super) fn maybe_auto_attach_clipboard_image(session: &mut Session) {
    if !session.staged_images.is_empty()
        || !crate::llm_client::provider_supports_vision(&session.config)
        || cfg!(test)
    {
        return;
    }

    let tmp = "/tmp/zap_auto_paste.png";
    if !crate::session::commands::paste_clipboard_image(tmp) || !std::path::Path::new(tmp).exists() {
        return;
    }

    if let Ok(bytes) = std::fs::read(tmp) {
        if bytes.len() >= 128 {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            let hash = hasher.finish();

            if session.last_auto_clip_hash != Some(hash) {
                use base64::Engine;
                let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let kb = bytes.len() / 1024;
                session.staged_images.push(("image/png".to_string(), data));
                session.last_auto_clip_hash = Some(hash);
                let msg = format!("✓ Clipboard image attached ({} KB).", kb);
                if crate::tui::channel::is_tui_mode() {
                    crate::tui::channel::tui_send(crate::tui::channel::TuiEvent::Notice(msg));
                } else {
                    println!("  {}", msg.dimmed());
                }
            }
            // Same image as a prior turn's auto-attach — the user hasn't
            // copied anything new, so don't resend it.
        }
        let _ = std::fs::remove_file(tmp);
    }
}
