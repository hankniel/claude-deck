import AppKit
import SwiftTerm
import SwiftUI

// MARK: - Theme

enum DeckTheme {
    static let accent = SwiftUI.Color(red: 0.80, green: 0.42, blue: 0.29)
    static let accentNS = NSColor(red: 0.80, green: 0.42, blue: 0.29, alpha: 1)

    private static func isDark(_ a: NSAppearance) -> Bool { a.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua }
    private static func dyn(_ light: UInt32, _ dark: UInt32) -> NSColor {
        NSColor(name: nil) { isDark($0) ? ns(dark) : ns(light) }
    }

    /// Validated categorical palette (fixed order; light/dark steps). Series colour follows the entity.
    static let series: [SwiftUI.Color] = [
        (0x2A78D6, 0x3987E5), (0xEB6834, 0xD95926), (0x1BAF7A, 0x199E70), (0xEDA100, 0xC98500),
        (0xE87BA4, 0xD55181), (0x008300, 0x008300), (0x6250D6, 0x9085E9), (0xE34948, 0xE66767),
    ].map { SwiftUI.Color(nsColor: dyn($0.0, $0.1)) }
    static let otherSeries = SwiftUI.Color(nsColor: dyn(0x9A9893, 0x6E6C67))

    static func usageColor(_ pct: Double, line: Double) -> SwiftUI.Color {
        pct >= line ? .red : pct >= line * 0.75 ? .orange : accent
    }

    static let terminalBackground = SwiftUI.Color(nsColor: dyn(0xFFFFFF, 0x1E1E1E))

    private static func c(_ hex: UInt32) -> SwiftTerm.Color {
        let r = UInt16((hex >> 16) & 0xff), g = UInt16((hex >> 8) & 0xff), b = UInt16(hex & 0xff)
        return SwiftTerm.Color(red: r * 257, green: g * 257, blue: b * 257)
    }
    static func ns(_ hex: UInt32) -> NSColor {
        NSColor(red: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
                blue: CGFloat(hex & 0xff) / 255, alpha: 1)
    }

    static func apply(to view: TerminalView, dark: Bool) {
        if dark {
            view.nativeBackgroundColor = ns(0x1E1E1E)
            view.nativeForegroundColor = ns(0xE6E6E3)
            view.installColors([0x3A3A38, 0xF0745F, 0x7CC27E, 0xE6BE5A, 0x6FA4E8, 0xC795E0, 0x63C3C0, 0xD4D4D0,
                                0x7A7A76, 0xFF8D78, 0x96D497, 0xF2CF76, 0x8DB9F0, 0xD8AEEB, 0x82D4D1, 0xF5F5F2].map(c))
            view.selectedTextBackgroundColor = NSColor(red: 0.85, green: 0.47, blue: 0.34, alpha: 0.35)
        } else {
            view.nativeBackgroundColor = ns(0xFFFFFF)
            view.nativeForegroundColor = ns(0x1D1D1F)
            view.installColors([0x1D1D1F, 0xC0392B, 0x2E7D32, 0x9A6700, 0x2563B8, 0x8E44AD, 0x16808A, 0xBDBDB8,
                                0x6E6E73, 0xD9534F, 0x3C9A40, 0xB07D12, 0x3B7DD8, 0xA35CC2, 0x1F99A3, 0x8E8E93].map(c))
            view.selectedTextBackgroundColor = NSColor(red: 0.79, green: 0.39, blue: 0.26, alpha: 0.22)
        }
        view.caretColor = accentNS
    }
}

// MARK: - Terminal view

/// Last time the process wrote output; set from SwiftTerm's I/O thread.
final class OutputClock: @unchecked Sendable {
    private let lock = NSLock()
    private var last = Date()
    func tick() { lock.lock(); last = Date(); lock.unlock() }
    var value: Date { lock.lock(); defer { lock.unlock() }; return last }
}

final class DeckTerminalView: LocalProcessTerminalView {
    let outputClock = OutputClock()
    var lastInput = Date()
    var lastOutput: Date { outputClock.value }

    func trackOutput() {
        let clock = outputClock
        setProcessOutputHandler { clock.tick() }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        DeckTheme.apply(to: self, dark: effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua)
    }
}

// MARK: - Shell integration

/// Makes `claude` in the app's shells go through `claude-deck launch` (account popup) and
/// reports the working directory with OSC 7. The user's own rc files are still loaded.
enum ShellIntegration {
    static var dir: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("CCDeck/shell")
    }

    static func install() {
        let zsh = dir.appendingPathComponent("zsh")
        try? FileManager.default.createDirectory(at: zsh, withIntermediateDirectories: true)
        let files: [String: String] = [
            ".zshenv": """
            _ccdeck_dir=$ZDOTDIR
            ZDOTDIR=${CC_DECK_USER_ZDOTDIR:-$HOME}
            [[ -f $ZDOTDIR/.zshenv ]] && source $ZDOTDIR/.zshenv
            _ccdeck_user=$ZDOTDIR
            ZDOTDIR=$_ccdeck_dir
            """,
            ".zprofile": """
            ZDOTDIR=$_ccdeck_user
            [[ -f $ZDOTDIR/.zprofile ]] && source $ZDOTDIR/.zprofile
            ZDOTDIR=$_ccdeck_dir
            """,
            ".zshrc": """
            ZDOTDIR=$_ccdeck_user
            [[ -f $ZDOTDIR/.zshrc ]] && source $ZDOTDIR/.zshrc
            unalias claude 2>/dev/null
            function claude { command "$CC_DECK_BIN" launch -- "$@"; }
            [[ ":$PATH:" != *":$CC_DECK_BIN_DIR:"* ]] && PATH="$PATH:$CC_DECK_BIN_DIR"
            _ccdeck_osc7() { printf '\\e]7;file://%s%s\\a' "${HOST}" "${PWD}"; }
            autoload -Uz add-zsh-hook 2>/dev/null && add-zsh-hook precmd _ccdeck_osc7
            _ccdeck_osc7
            """,
        ]
        for (name, body) in files {
            try? (body + "\n").write(to: zsh.appendingPathComponent(name), atomically: true, encoding: .utf8)
        }
        let bash = """
        [ -f ~/.bash_profile ] && . ~/.bash_profile || { [ -f ~/.bashrc ] && . ~/.bashrc; }
        unalias claude 2>/dev/null
        claude() { command "$CC_DECK_BIN" launch -- "$@"; }
        case ":$PATH:" in *":$CC_DECK_BIN_DIR:"*) ;; *) PATH="$PATH:$CC_DECK_BIN_DIR";; esac
        PROMPT_COMMAND='printf "\\e]7;file://%s%s\\a" "$HOSTNAME" "$PWD"'"${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
        """
        try? (bash + "\n").write(to: dir.appendingPathComponent("bashrc"), atomically: true, encoding: .utf8)
    }

    static var userShell: String {
        if let pw = getpwuid(getuid()), let sh = pw.pointee.pw_shell { return String(cString: sh) }
        return ProcessInfo.processInfo.environment["SHELL"] ?? "/bin/zsh"
    }

    /// (executable, args, environment, argv0)
    static func launchSpec(socket: String, tabID: UUID) -> (String, [String], [String], String) {
        let shell = userShell
        let name = (shell as NSString).lastPathComponent
        var env = ProcessInfo.processInfo.environment
        env["TERM"] = "xterm-256color"
        env["COLORTERM"] = "truecolor"
        env["TERM_PROGRAM"] = "CCDeck"
        env["LANG"] = env["LANG"] ?? "en_US.UTF-8"
        env["CC_DECK_SOCKET"] = socket
        env["CC_DECK_TAB"] = tabID.uuidString
        env["CC_DECK_BIN"] = DeckCLI.binary
        env["CC_DECK_BIN_DIR"] = DeckCLI.binDir
        env.removeValue(forKey: "CLAUDE_CONFIG_DIR")
        var args: [String] = []
        switch name {
        case "zsh":
            env["CC_DECK_USER_ZDOTDIR"] = env["ZDOTDIR"] ?? NSHomeDirectory()
            env["ZDOTDIR"] = dir.appendingPathComponent("zsh").path
            args = ["-l"]
        case "bash":
            args = ["--rcfile", dir.appendingPathComponent("bashrc").path, "-i"]
        default:
            args = ["-l"]
        }
        return (shell, args, env.map { "\($0.key)=\($0.value)" }, "-" + name)
    }
}

// MARK: - Tab model

struct ClaudeRun: Equatable {
    var account: String
    var sessionID: String?
    var cwd: String
}

@MainActor
final class TerminalTab: NSObject, ObservableObject, Identifiable, LocalProcessTerminalViewDelegate {
    let id = UUID()
    let view: DeckTerminalView
    @Published var title: String = ""
    @Published var cwd: String
    @Published var claude: ClaudeRun?
    @Published var exited = false
    var connection: DeckConnection?
    var lastSwitch: Date?
    var warned = false
    var onExit: ((TerminalTab) -> Void)?

    init(cwd: String, socket: String) {
        self.cwd = cwd
        view = DeckTerminalView(frame: NSRect(x: 0, y: 0, width: 900, height: 600))
        super.init()
        view.font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        view.optionAsMetaKey = true
        let dark = NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        DeckTheme.apply(to: view, dark: dark)
        view.processDelegate = self
        view.trackOutput()
        let (exe, args, env, argv0) = ShellIntegration.launchSpec(socket: socket, tabID: id)
        view.startProcess(executable: exe, args: args, environment: env, execName: argv0, currentDirectory: cwd)
    }

    var displayTitle: String {
        if let c = claude { return title.isEmpty ? "Claude · \(c.account)" : title }
        if !title.isEmpty { return title }
        return cwd == NSHomeDirectory() ? "Home" : (cwd as NSString).lastPathComponent
    }

    var subtitle: String {
        let dir = (claude?.cwd ?? cwd) as NSString
        if let c = claude { return "\(c.account) · \(dir.lastPathComponent)" }
        return cwd == NSHomeDirectory() ? "Terminal" : DeckFormat.shortPath(cwd)
    }

    var idleFor: TimeInterval {
        min(Date().timeIntervalSince(view.lastOutput), Date().timeIntervalSince(view.lastInput))
    }

    func type(_ text: String) { view.send(txt: text) }

    func terminate() { view.terminate() }

    // LocalProcessTerminalViewDelegate
    nonisolated func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {}

    nonisolated func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        let t = title.trimmingCharacters(in: .whitespaces)
            .drop(while: { !$0.isLetter && !$0.isNumber })
        Task { @MainActor in self.title = String(t) }
    }

    nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        guard let d = directory else { return }
        let path = URL(string: d)?.path ?? d
        Task { @MainActor in if !path.isEmpty { self.cwd = path } }
    }

    nonisolated func processTerminated(source: TerminalView, exitCode: Int32?) {
        Task { @MainActor in
            self.exited = true
            self.onExit?(self)
        }
    }
}

/// Hosts the selected tab's terminal view; views live on their tabs so shells keep running.
struct TerminalHost: NSViewRepresentable {
    let tab: TerminalTab

    func makeNSView(context: Context) -> NSView {
        let v = NSView()
        v.wantsLayer = true
        return v
    }

    func updateNSView(_ container: NSView, context: Context) {
        guard container.subviews.first !== tab.view else { return }
        container.subviews.forEach { $0.removeFromSuperview() }
        tab.view.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(tab.view)
        NSLayoutConstraint.activate([
            tab.view.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            tab.view.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -6),
            tab.view.topAnchor.constraint(equalTo: container.topAnchor, constant: 10),
            tab.view.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -6),
        ])
        DispatchQueue.main.async { tab.view.window?.makeFirstResponder(tab.view) }
    }
}
