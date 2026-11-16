//! Which AI coding tools are installed on this machine, and which of them
//! Quota can meter. Detection only checks whether a tool's own files exist
//! (no file is opened, nothing is sent anywhere).
//!
//! A tool is "metered" only when its usage source was verified on Windows
//! (see each provider module). Everything else is reported honestly as
//! detected-but-not-metered rather than guessed.

use std::path::PathBuf;

/// Where a tool leaves files.
#[derive(Debug, Clone, Copy)]
enum Root {
    Home,
    Roaming,
    Local,
    /// A VS Code-family extension's globalStorage folder (`publisher.name`).
    VsExt,
}

#[derive(Debug, Clone, Copy)]
pub struct Tool {
    pub id: &'static str,
    pub name: &'static str,
    /// Provider id when Quota can meter this tool.
    pub provider: Option<&'static str>,
    marks: &'static [(Root, &'static str)],
}

use Root::*;

/// 25 AI coding tools, metered ones first.
pub const TOOLS: &[Tool] = &[
    Tool { id: "claude", name: "Claude Code", provider: Some("claude"), marks: &[(Home, ".claude/.credentials.json")] },
    Tool { id: "codex", name: "Codex", provider: Some("codex"), marks: &[(Home, ".codex/auth.json")] },
    Tool { id: "cursor", name: "Cursor", provider: Some("cursor"), marks: &[(Roaming, "Cursor/User/globalStorage/state.vscdb")] },
    Tool {
        id: "copilot",
        name: "GitHub Copilot",
        provider: Some("copilot"),
        marks: &[(Local, "github-copilot"), (VsExt, "github.copilot-chat"), (Home, ".copilot")],
    },
    Tool { id: "openrouter", name: "OpenRouter", provider: Some("openrouter"), marks: &[] },
    Tool { id: "gemini", name: "Gemini CLI", provider: None, marks: &[(Home, ".gemini/oauth_creds.json")] },
    Tool { id: "antigravity", name: "Antigravity", provider: None, marks: &[(Roaming, "Antigravity"), (Home, ".gemini/antigravity")] },
    Tool { id: "windsurf", name: "Windsurf", provider: None, marks: &[(Roaming, "Windsurf"), (Home, ".codeium/windsurf")] },
    Tool { id: "zed", name: "Zed", provider: None, marks: &[(Roaming, "Zed"), (Local, "Zed")] },
    Tool { id: "factory", name: "Factory Droid", provider: None, marks: &[(Home, ".factory")] },
    Tool { id: "opencode", name: "OpenCode", provider: None, marks: &[(Home, ".local/share/opencode")] },
    Tool { id: "grok", name: "Grok CLI", provider: None, marks: &[(Home, ".grok/auth.json")] },
    Tool { id: "kiro", name: "Kiro", provider: None, marks: &[(Roaming, "Kiro"), (Home, ".kiro")] },
    Tool {
        id: "amazonq",
        name: "Amazon Q Developer",
        provider: None,
        marks: &[(VsExt, "amazonwebservices.amazon-q-vscode"), (Home, ".aws/amazonq")],
    },
    Tool { id: "cline", name: "Cline", provider: None, marks: &[(VsExt, "saoudrizwan.claude-dev"), (Home, ".cline")] },
    Tool { id: "roo", name: "Roo Code", provider: None, marks: &[(VsExt, "rooveterinaryinc.roo-cline")] },
    Tool { id: "kilo", name: "Kilo Code", provider: None, marks: &[(VsExt, "kilocode.kilo-code")] },
    Tool { id: "continue", name: "Continue", provider: None, marks: &[(Home, ".continue")] },
    Tool { id: "augment", name: "Augment Code", provider: None, marks: &[(Home, ".augment"), (VsExt, "augment.vscode-augment")] },
    Tool { id: "tabnine", name: "Tabnine", provider: None, marks: &[(VsExt, "tabnine.tabnine-vscode"), (Roaming, "TabNine")] },
    Tool { id: "trae", name: "Trae", provider: None, marks: &[(Roaming, "Trae"), (Home, ".trae")] },
    Tool { id: "qwen", name: "Qwen Code", provider: None, marks: &[(Home, ".qwen")] },
    Tool { id: "amp", name: "Amp", provider: None, marks: &[(Home, ".config/amp"), (VsExt, "sourcegraph.amp")] },
    Tool { id: "zencoder", name: "Zencoder", provider: None, marks: &[(VsExt, "zencoderai.zencoder")] },
    Tool { id: "blackbox", name: "Blackbox AI", provider: None, marks: &[(VsExt, "blackboxapp.blackbox"), (VsExt, "blackboxapp.blackboxagent")] },
];

/// VS Code-family editors whose globalStorage may hold extension state.
const VS_EDITORS: [&str; 4] = ["Code", "Code - Insiders", "Cursor", "Windsurf"];

fn candidates(root: Root, rel: &str) -> Vec<PathBuf> {
    let base = match root {
        Home => crate::paths::home(),
        Roaming => crate::paths::roaming(),
        Local => crate::paths::local(),
        VsExt => {
            return crate::paths::roaming()
                .map(|r| {
                    VS_EDITORS.iter().map(|e| r.join(e).join("User").join("globalStorage").join(rel)).collect()
                })
                .unwrap_or_default()
        }
    };
    base.map(|b| vec![b.join(rel)]).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub id: &'static str,
    pub name: &'static str,
    pub installed: bool,
    pub provider: Option<&'static str>,
}

impl Detected {
    /// `metered` | `not metered yet` | `not found`
    pub fn status(&self) -> &'static str {
        match (self.installed, self.provider.is_some()) {
            (_, true) => "metered",
            (true, false) => "not metered yet",
            (false, false) => "not found",
        }
    }
}

fn openrouter_present() -> bool {
    std::env::var_os("OPENROUTER_API_KEY").is_some_and(|v| !v.is_empty())
        || super::opencode_auth().is_some_and(|v| v.get("openrouter").is_some())
}

/// Scans the machine. Cheap: a few dozen `exists()` checks plus one small
/// JSON read for OpenRouter.
pub fn scan() -> Vec<Detected> {
    TOOLS
        .iter()
        .map(|t| {
            let installed = if t.id == "openrouter" {
                openrouter_present()
            } else {
                t.marks.iter().any(|(root, rel)| candidates(*root, rel).iter().any(|p| p.exists()))
            };
            Detected { id: t.id, name: t.name, installed, provider: t.provider }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_25_unique_tools_and_providers_exist() {
        assert_eq!(TOOLS.len(), 25);
        let mut ids: Vec<_> = TOOLS.iter().map(|t| t.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 25);
        for t in TOOLS {
            if let Some(p) = t.provider {
                assert!(super::super::ALL_IDS.contains(&p), "{p}");
            }
        }
        // Every provider is in the catalog.
        for id in super::super::ALL_IDS {
            assert!(TOOLS.iter().any(|t| t.provider == Some(id)), "{id}");
        }
    }

    #[test]
    fn candidate_paths_and_status() {
        let home = candidates(Home, ".qwen");
        assert!(home.len() <= 1 && home.iter().all(|p| p.ends_with(".qwen")));
        // Extensions are looked up in every VS Code-family editor.
        let ext = candidates(VsExt, "kilocode.kilo-code");
        assert!(ext.is_empty() || ext.len() == VS_EDITORS.len());
        assert!(ext.iter().all(|p| p.ends_with("globalStorage/kilocode.kilo-code")));
        let d = Detected { id: "x", name: "X", installed: true, provider: None };
        assert_eq!(d.status(), "not metered yet");
        assert_eq!(Detected { installed: false, ..d.clone() }.status(), "not found");
        assert_eq!(Detected { provider: Some("claude"), ..d }.status(), "metered");
    }
}
