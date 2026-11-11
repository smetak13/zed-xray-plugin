use zed_extension_api::{self as zed, Command, LanguageServerId, Result, Worktree};

const BINARY_NOT_FOUND: &str =
    "xray binary not found: set `lsp.xray.binary.path` in Zed settings or put `xray` on PATH";

struct XrayExtension;

impl zed::Extension for XrayExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        let command = worktree
            .which("xray")
            .ok_or_else(|| BINARY_NOT_FOUND.to_string())?;
        Ok(Command {
            command,
            args: Vec::new(),
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(XrayExtension);
