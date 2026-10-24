// The desktop tools a session without DevTools runs, as admin/src/tools.json pins them. Written by
// scripts/tools.py; scripts/check.sh fails when the two differ.
export const TOOLS: readonly string[] = ['list_windows', 'get_window_state', 'click', 'type_text', 'press_key', 'hotkey', 'scroll', 'get_desktop_state', 'run_steps', 'run_script'];
