use std::collections::BTreeMap;
use zellij_tile::prelude::run_command;

use crate::installer::{tag_script, HOOK_VERSION_TAG};

fn hook_script_content() -> String {
    tag_script(include_str!("../scripts/codex-hook.sh"))
}

fn session_event_script_content() -> String {
    tag_script(include_str!("../scripts/codex-session-event.sh"))
}

const HOOKS_BLOCK_BEGIN: &str = "# BEGIN ZELLAUDE HOOKS";
const HOOKS_BLOCK_END: &str = "# END ZELLAUDE HOOKS";

const INSTALL_TEMPLATE: &str = r##"set -e
HOOK_PATH="$HOME/.config/zellij/plugins/codex-hook.sh"
HOOK_CMD="$HOME/.config/zellij/plugins/codex-hook.sh"
EVENT_PATH="$HOME/.config/zellij/plugins/codex-session-event.sh"
CONFIG_DIR="$HOME/.codex"
CONFIG_FILE="$CONFIG_DIR/config.toml"

# Skip if Codex isn't on this machine and the user hasn't set up a config dir.
if ! command -v codex >/dev/null 2>&1 && [ ! -d "$CONFIG_DIR" ]; then
  echo "no_codex"
  exit 0
fi

# Resolve symlink so `mv $tmp $CONFIG_FILE` rewrites the link target instead
# of replacing a user's symlink with a regular file (e.g. dotfile setups
# where ~/.codex/config.toml -> ~/.dotfiles/codex/config.toml).
if [ -L "$CONFIG_FILE" ]; then
  CONFIG_FILE="$(readlink -f "$CONFIG_FILE")"
fi

# Idempotency: if both helper scripts and the managed block are at the
# current version, do nothing.
if grep -qF '__VERSION_TAG__' "$HOOK_PATH" 2>/dev/null \
   && grep -qF '__VERSION_TAG__' "$EVENT_PATH" 2>/dev/null \
   && [ -f "$CONFIG_FILE" ] \
   && grep -qF '__BLOCK_BEGIN__' "$CONFIG_FILE" 2>/dev/null \
   && grep -qF "$HOOK_CMD" "$CONFIG_FILE" 2>/dev/null; then
  echo "current"
  exit 0
fi

mkdir -p "$(dirname "$HOOK_PATH")"

# Write hook script
cat > "$HOOK_PATH" << 'CODEX_HOOK_EOF'
__HOOK_SCRIPT__
CODEX_HOOK_EOF
chmod +x "$HOOK_PATH"

# Write session-event helper (called from user's cdx/codex shell wrapper)
cat > "$EVENT_PATH" << 'CODEX_EVENT_EOF'
__EVENT_SCRIPT__
CODEX_EVENT_EOF
chmod +x "$EVENT_PATH"

mkdir -p "$CONFIG_DIR"
[ ! -f "$CONFIG_FILE" ] && touch "$CONFIG_FILE"
cp "$CONFIG_FILE" "$CONFIG_FILE.bak"

# Step 1: ensure [features] codex_hooks = true.
# When a [features] table is found, codex_hooks = true is emitted immediately
# after the header and any pre-existing codex_hooks line is dropped (idempotent
# replace). When no [features] table exists, the section is appended at EOF.
tmp=$(mktemp)
awk '
BEGIN { in_features = 0; features_seen = 0 }
/^\[features\][[:space:]]*$/ {
  in_features = 1
  features_seen = 1
  print
  print "codex_hooks = true"
  next
}
/^\[/ { in_features = 0; print; next }
in_features && /^[[:space:]]*codex_hooks[[:space:]]*=/ { next }
{ print }
END {
  if (!features_seen) {
    if (NR > 0) print ""
    print "[features]"
    print "codex_hooks = true"
  }
}
' "$CONFIG_FILE" > "$tmp" && mv "$tmp" "$CONFIG_FILE"

# Step 2: replace the managed [[hooks.X]] block (lines between markers).
tmp=$(mktemp)
awk -v begin='__BLOCK_BEGIN__' -v end='__BLOCK_END__' '
{
  if ($0 == begin) { skip = 1; next }
  if ($0 == end)   { skip = 0; next }
  if (!skip) print
}
' "$CONFIG_FILE" > "$tmp" && mv "$tmp" "$CONFIG_FILE"

# Trim trailing blank lines for clean append
tmp=$(mktemp)
awk 'NF { for (i=1; i<=blank; i++) print ""; blank=0; print; next } { blank++ }' \
  "$CONFIG_FILE" > "$tmp" && mv "$tmp" "$CONFIG_FILE"

# Append fresh hook block
{
  [ -s "$CONFIG_FILE" ] && printf '\n'
  printf '%s\n' '__BLOCK_BEGIN__'
  cat << CODEX_BLOCK_EOF
[[hooks.SessionStart]]
matcher = "startup|resume"
[[hooks.SessionStart.hooks]]
type = "command"
command = "$HOOK_CMD"

[[hooks.UserPromptSubmit]]
[[hooks.UserPromptSubmit.hooks]]
type = "command"
command = "$HOOK_CMD"

[[hooks.PreToolUse]]
[[hooks.PreToolUse.hooks]]
type = "command"
command = "$HOOK_CMD"

[[hooks.PostToolUse]]
[[hooks.PostToolUse.hooks]]
type = "command"
command = "$HOOK_CMD"

[[hooks.PermissionRequest]]
[[hooks.PermissionRequest.hooks]]
type = "command"
command = "$HOOK_CMD"

[[hooks.Stop]]
[[hooks.Stop.hooks]]
type = "command"
command = "$HOOK_CMD"
CODEX_BLOCK_EOF
  printf '%s\n' '__BLOCK_END__'
} >> "$CONFIG_FILE"

echo "installed"
"##;

/// Idempotent installer for Codex CLI hooks. Detects Codex presence, writes
/// the normalizing hook script, enables `features.codex_hooks` in
/// `~/.codex/config.toml`, and registers hook commands for the six
/// lifecycle events used by the plugin.
pub fn run_install() {
    let cmd = INSTALL_TEMPLATE
        .replace("__VERSION_TAG__", HOOK_VERSION_TAG)
        .replace("__BLOCK_BEGIN__", HOOKS_BLOCK_BEGIN)
        .replace("__BLOCK_END__", HOOKS_BLOCK_END)
        .replace("__HOOK_SCRIPT__", &hook_script_content())
        .replace("__EVENT_SCRIPT__", &session_event_script_content());

    let mut ctx = BTreeMap::new();
    ctx.insert("type".into(), "install_codex_hooks".into());
    run_command(&["sh", "-c", &cmd], ctx);
}
