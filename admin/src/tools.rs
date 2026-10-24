//! The driver tools an agent token may call: the desktop tools a default session (no DevTools) runs, pinned from the
//! driver's own `tools/list` in `tools.json` (from the release the agent image pins; `scripts/tools.py` writes it).
//! The agents end-to-end test fails when the live driver's definitions of them differ.

use std::sync::LazyLock;

use serde_json::Value;

/// The largest request body a tool call may send, to the agent API and to `/mcp`. JSON may write each code point of a
/// string as two `\u` escapes (12 bytes, a surrogate pair), so this fits the largest pinned `maxLength` (`run_script`'s
/// script, 65,536) so written, with room for the other arguments and the MCP envelope.
pub const BODY_LIMIT: usize = 1024 * 1024;

static SNAPSHOT: LazyLock<Vec<Value>> = LazyLock::new(|| {
    let file: Value = serde_json::from_str(include_str!("tools.json")).expect("tools.json is JSON");
    file["tools"].as_array().expect("tools.json lists tools").clone()
});

/// Each tool as the driver defines it: `name`, `description`, `inputSchema` and `annotations`.
pub fn driver() -> &'static [Value] {
    &SNAPSHOT
}

pub fn known(name: &str) -> bool {
    SNAPSHOT.iter().any(|t| t["name"] == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_holds_desktop_tools_only_each_with_an_object_schema() {
        assert!(known("click") && known("get_desktop_state") && !known("browser_navigate") && !known("launch_app"));
        for tool in driver() {
            let name = tool["name"].as_str().expect("a name");
            assert!(!name.starts_with("browser_"), "{name} needs DevTools");
            assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
            assert!(tool["description"].is_string(), "{name}");
        }
    }

    #[test]
    fn the_body_limit_fits_the_largest_pinned_argument_written_as_escapes() {
        fn longest(schema: &Value) -> u64 {
            let own = schema.get("maxLength").and_then(Value::as_u64).unwrap_or(0);
            let nested = match schema {
                Value::Object(map) => map.values().map(longest).max(),
                Value::Array(items) => items.iter().map(longest).max(),
                _ => None,
            };
            own.max(nested.unwrap_or(0))
        }
        let longest = driver().iter().map(|t| longest(&t["inputSchema"])).max().unwrap();
        assert_eq!(longest, 65_536, "run_script's script");
        assert!(longest as usize * 12 + 64 * 1024 <= BODY_LIMIT);
    }
}
