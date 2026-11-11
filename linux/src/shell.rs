//! Shell integration: in CC Deck tabs, `claude` goes through `claude-deck launch` (account
//! picker + movable session) and the shell reports its directory with OSC 7.
//! The user's own rc files are still loaded.

use claude_deck::config::{data_dir, home};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn dir() -> PathBuf {
    data_dir().join("linux-shell")
}

fn is_exec(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// The `claude-deck` CLI: $CC_DECK_BIN, next to this binary, ~/.cargo/bin, ~/.local/bin, then PATH.
pub fn cli_binary() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = vec![];
    if let Some(p) = std::env::var_os("CC_DECK_BIN") {
        cands.push(p.into());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            cands.push(d.join("claude-deck"));
        }
    }
    cands.push(home().join(".cargo/bin/claude-deck"));
    cands.push(home().join(".local/bin/claude-deck"));
    if let Some(path) = std::env::var_os("PATH") {
        cands.extend(std::env::split_paths(&path).map(|d| d.join("claude-deck")));
    }
    cands.into_iter().find(|p| is_exec(p))
}

pub fn install() {
    let d = dir();
    let zsh = d.join("zsh");
    let _ = std::fs::create_dir_all(&zsh);
    let files: &[(&str, &str)] = &[
        (
            "zsh/.zshenv",
            r#"_ccdeck_dir=$ZDOTDIR
ZDOTDIR=${CC_DECK_USER_ZDOTDIR:-$HOME}
[[ -f $ZDOTDIR/.zshenv ]] && source $ZDOTDIR/.zshenv
_ccdeck_user=$ZDOTDIR
ZDOTDIR=$_ccdeck_dir
"#,
        ),
        (
            "zsh/.zprofile",
            r#"ZDOTDIR=$_ccdeck_user
[[ -f $ZDOTDIR/.zprofile ]] && source $ZDOTDIR/.zprofile
ZDOTDIR=$_ccdeck_dir
"#,
        ),
        (
            "zsh/.zshrc",
            r#"ZDOTDIR=$_ccdeck_user
[[ -f $ZDOTDIR/.zshrc ]] && source $ZDOTDIR/.zshrc
if [[ -x $CC_DECK_BIN ]]; then
  unalias claude 2>/dev/null
  function claude { command "$CC_DECK_BIN" launch -- "$@"; }
  [[ ":$PATH:" != *":$CC_DECK_BIN_DIR:"* ]] && PATH="$PATH:$CC_DECK_BIN_DIR"
fi
_ccdeck_osc7() { printf '\e]7;file://%s%s\a' "${HOST}" "${PWD}"; }
autoload -Uz add-zsh-hook 2>/dev/null && add-zsh-hook precmd _ccdeck_osc7
_ccdeck_osc7
"#,
        ),
        (
            "bashrc",
            r#"[ -f ~/.bashrc ] && . ~/.bashrc
if [ -x "$CC_DECK_BIN" ]; then
  unalias claude 2>/dev/null
  claude() { command "$CC_DECK_BIN" launch -- "$@"; }
  case ":$PATH:" in *":$CC_DECK_BIN_DIR:"*) ;; *) PATH="$PATH:$CC_DECK_BIN_DIR";; esac
fi
_ccdeck_osc7() { printf '\e]7;file://%s%s\a' "$HOSTNAME" "$PWD"; }
PROMPT_COMMAND="_ccdeck_osc7${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
"#,
        ),
        (
            "config.fish",
            r#"if test -x "$CC_DECK_BIN"
  functions -e claude 2>/dev/null
  function claude; command "$CC_DECK_BIN" launch -- $argv; end
  contains -- "$CC_DECK_BIN_DIR" $PATH; or set -gx PATH $PATH "$CC_DECK_BIN_DIR"
end
function _ccdeck_osc7 --on-event fish_prompt; printf '\e]7;file://%s%s\a' (hostname) "$PWD"; end
"#,
        ),
    ];
    for (name, body) in files {
        let _ = std::fs::write(d.join(name), body);
    }
}

pub fn user_shell() -> String {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            let s = std::ffi::CStr::from_ptr((*pw).pw_shell).to_string_lossy().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
}

pub struct Spec {
    pub argv: Vec<String>,
    /// Added to (or overriding) the app's own environment.
    pub env: Vec<String>,
}

pub fn spec(socket: &Path, tab_id: &str, cli: Option<&Path>) -> Spec {
    let shell = user_shell();
    let name = Path::new(&shell).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let cli_s = cli.map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let cli_dir = cli
        .and_then(|p| p.parent())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut env = vec![
        "TERM=xterm-256color".to_string(),
        "COLORTERM=truecolor".to_string(),
        "TERM_PROGRAM=CCDeck".to_string(),
        format!("CC_DECK_SOCKET={}", socket.display()),
        format!("CC_DECK_TAB={tab_id}"),
        format!("CC_DECK_BIN={cli_s}"),
        format!("CC_DECK_BIN_DIR={cli_dir}"),
    ];
    let d = dir();
    let argv = match name.as_str() {
        "zsh" => {
            let user = std::env::var("ZDOTDIR").unwrap_or_else(|_| home().to_string_lossy().to_string());
            env.push(format!("CC_DECK_USER_ZDOTDIR={user}"));
            env.push(format!("ZDOTDIR={}", d.join("zsh").display()));
            vec![shell, "-l".into()]
        }
        "bash" => vec![shell, "--rcfile".into(), d.join("bashrc").to_string_lossy().to_string(), "-i".into()],
        "fish" => vec![
            shell,
            "-l".into(),
            "-C".into(),
            format!("source {}", d.join("config.fish").display()),
        ],
        _ => vec![shell, "-l".into()],
    };
    Spec { argv, env }
}
