//! This Mac's overlay host name, from the environment or the platform. The rules are in
//! `overlay::host_from`; this file only gathers the inputs.

use super::overlay;

/// This Mac's overlay host name: `$FLICK_HOST_NAME`, else its `LocalHostName`, lowercase.
pub fn host_name() -> Option<String> {
    let env = std::env::var_os("FLICK_HOST_NAME").map(|v| v.to_string_lossy().into_owned());
    overlay::host_from(env.as_deref(), &crate::platform::host::local_host_name)
}
