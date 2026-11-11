import AppKit
import SwiftUI
import UserNotifications

/// Window appearance chosen in Settings: follow macOS, or force light/dark.
enum AppAppearance: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    static let key = "appearance"

    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }

    static var current: AppAppearance {
        AppAppearance(rawValue: UserDefaults.standard.string(forKey: key) ?? "") ?? .system
    }

    @MainActor
    static func apply(_ a: AppAppearance) {
        switch a {
        case .system: NSApp.appearance = nil
        case .light: NSApp.appearance = NSAppearance(named: .aqua)
        case .dark: NSApp.appearance = NSAppearance(named: .darkAqua)
        }
    }
}

enum SidebarItem: Hashable {
    case tab(UUID)
    case dashboard
}

/// A `claude` typed in a tab, waiting for the user to choose its account.
struct PickRequest: Identifiable {
    let id = UUID()
    let tabID: UUID?
    let cwd: String
    let args: [String]
    let connection: DeckConnection
}

@MainActor
final class AppState: ObservableObject {
    @Published var tabs: [TerminalTab] = []
    @Published var selection: SidebarItem?
    @Published var accounts: [AccountInfo] = []
    @Published var config = DeckConfig(threshold: 90, weekly_threshold: 95, auto_switch: false,
                                       idle_secs: 8, cooldown_secs: 300, poll_secs: 120, maps: [:])
    @Published var pick: PickRequest?
    @Published var notice: String?
    @Published var lastRefresh: Date?
    @Published var refreshing = false
    @Published var columns: NavigationSplitViewVisibility = .all

    private var pickQueue: [PickRequest] = []
    private let server = ControlServer()
    private var timers: [Timer] = []
    private var keyMonitor: Any?
    private var lastRecord = Date.distantPast
    private var noticeTask: Task<Void, Never>?

    init() {
        ShellIntegration.install()
        server.onMessage = { [weak self] conn, msg in self?.handle(conn, msg) }
        server.onClose = { [weak self] conn in self?.connectionClosed(conn) }
        do { try server.start() } catch { flash("Shell integration unavailable: \(error.localizedDescription)") }
        newTab()
        Task { await refresh(record: true) }
        timers.append(Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.autoSwitchTick() }
        })
        timers.append(Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                if Date().timeIntervalSince(self.lastRefresh ?? .distantPast) >= Double(max(30, self.config.poll_secs)) {
                    await self.refresh(record: true)
                }
            }
        })
        installKeyMonitor()
        AppAppearance.apply(AppAppearance.current)
        snapshotIfRequested()
        if Bundle.main.bundleIdentifier != nil, ProcessInfo.processInfo.environment["CC_DECK_SNAPSHOT"] == nil {
            UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
        }
    }

    func shutdown() {
        tabs.forEach { $0.terminate() }
        server.stop()
    }

    // MARK: snapshot (development: renders the window to PNG without screen-recording permission)

    private func snapshotIfRequested() {
        let env = ProcessInfo.processInfo.environment
        guard let out = env["CC_DECK_SNAPSHOT"] else { return }
        let mode = env["CC_DECK_SNAPSHOT_MODE"] ?? "terminal"
        if let a = env["CC_DECK_APPEARANCE"] { NSApp.appearance = NSAppearance(named: a == "dark" ? .darkAqua : .aqua) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [self] in
            switch mode {
            case "dashboard": selection = .dashboard
            case "settings": NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
            case "home": tabs.forEach { close($0) }
            case "picker":
                pick = PickRequest(tabID: activeTab?.id, cwd: NSHomeDirectory() + "/Projects/claude-deck",
                                   args: [], connection: DeckConnection(fd: -1))
            case "claude":
                activeTab?.claude = ClaudeRun(account: accounts.first?.name ?? "default", sessionID: nil,
                                              cwd: NSHomeDirectory() + "/Projects/claude-deck")
                activeTab?.title = "Refactor usage poller"
                newTab()
                select(index: 0)
            default: break
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                for (i, w) in NSApp.windows.enumerated() where w.isVisible && w.frame.width > 300 {
                    Self.capture(w, to: out.replacingOccurrences(of: ".png", with: i == 0 ? ".png" : "-\(i).png"))
                    if let sheet = w.attachedSheet { Self.capture(sheet, to: out.replacingOccurrences(of: ".png", with: "-sheet.png")) }
                }
                // A modal sheet can block NSApp.terminate; snapshot runs must always exit.
                self.shutdown()
                exit(0)
            }
        }
    }

    private static func capture(_ w: NSWindow, to path: String) {
        guard let view = w.contentView?.superview ?? w.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
    }

    // MARK: tabs

    var activeTab: TerminalTab? {
        if case .tab(let id) = selection { return tabs.first { $0.id == id } }
        return nil
    }

    func newTab(cwd: String? = nil, run command: String? = nil) {
        let dir = cwd ?? activeTab?.cwd ?? NSHomeDirectory()
        let tab = TerminalTab(cwd: dir, socket: server.path)
        tab.onExit = { [weak self] t in self?.close(t) }
        tabs.append(tab)
        selection = .tab(tab.id)
        if let command { DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) { tab.type(command + "\r") } }
    }

    func close(_ tab: TerminalTab) {
        guard let i = tabs.firstIndex(where: { $0.id == tab.id }) else { return }
        if !tab.exited { tab.terminate() }
        tabs.remove(at: i)
        if case .tab(let id) = selection, id == tab.id {
            selection = tabs.isEmpty ? nil : .tab(tabs[min(i, tabs.count - 1)].id)
        }
    }

    func closeActive() {
        if let t = activeTab { close(t) }
    }

    func select(index: Int) {
        guard tabs.indices.contains(index) else { return }
        selection = .tab(tabs[index].id)
    }

    func cycle(_ delta: Int) {
        guard !tabs.isEmpty else { return }
        let i = activeTab.flatMap { t in tabs.firstIndex { $0.id == t.id } } ?? 0
        select(index: (i + delta + tabs.count) % tabs.count)
    }

    func startClaudeInActiveTab() {
        if activeTab == nil { newTab() }
        activeTab?.type("claude\r")
    }

    /// Shift-Return inserts a newline in Claude Code's prompt (ESC CR), and typing counts as activity.
    private func installKeyMonitor() {
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, let term = event.window?.firstResponder as? DeckTerminalView else { return event }
            term.lastInput = Date()
            let mods = event.modifierFlags.intersection([.shift, .command, .control, .option])
            if event.keyCode == 36 || event.keyCode == 76, mods == .shift {
                term.send(txt: "\u{1b}\r")
                return nil
            }
            _ = self
            return event
        }
    }

    // MARK: usage & settings

    func refresh(record: Bool = false) async {
        refreshing = true
        defer { refreshing = false }
        do {
            let s = try await DeckCLI.state(record: record)
            accounts = s.accounts
            config = s.config
            lastRefresh = Date()
        } catch {
            flash("Couldn't read accounts: \(error.localizedDescription)")
        }
    }

    func account(_ name: String) -> AccountInfo? { accounts.first { $0.name == name } }

    func best(excluding: String?) -> AccountInfo? {
        accounts
            .filter { $0.name != excluding && !$0.disabled && $0.isUsable }
            .min { ($0.pressure ?? 9) < ($1.pressure ?? 9) }
    }

    func setConfig(_ key: String, _ value: String) {
        Task {
            do { try await DeckCLI.set(key, value) } catch { flash(error.localizedDescription) }
            await refresh()
        }
    }

    func flash(_ text: String) {
        notice = text
        noticeTask?.cancel()
        noticeTask = Task {
            try? await Task.sleep(for: .seconds(6))
            if !Task.isCancelled { notice = nil }
        }
    }

    func notify(_ title: String, _ body: String) {
        guard Bundle.main.bundleIdentifier != nil else { return }
        let c = UNMutableNotificationContent()
        c.title = title
        c.body = body
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: c, trigger: nil))
    }

    // MARK: launch protocol

    private func tab(for conn: DeckConnection) -> TerminalTab? { tabs.first { $0.connection === conn } }

    private func handle(_ conn: DeckConnection, _ msg: [String: Any]) {
        switch msg["type"] as? String {
        case "hello":
            let tabID = (msg["tab"] as? String).flatMap(UUID.init(uuidString:))
            let tab = tabs.first { $0.id == tabID }
            tab?.connection = conn
            if msg["pick"] as? Bool == true {
                let req = PickRequest(tabID: tabID, cwd: msg["cwd"] as? String ?? tab?.cwd ?? NSHomeDirectory(),
                                      args: msg["args"] as? [String] ?? [], connection: conn)
                if let tab { selection = .tab(tab.id) }
                if pick == nil { pick = req } else { pickQueue.append(req) }
                NSApp.activate(ignoringOtherApps: true)
                Task { await refresh() }
            }
        case "started":
            guard let tab = tab(for: conn), let account = msg["account"] as? String else { return }
            tab.claude = ClaudeRun(account: account, sessionID: msg["session_id"] as? String,
                                   cwd: msg["cwd"] as? String ?? tab.cwd)
            tab.warned = false
            Task { await refresh() }
        case "exited":
            tab(for: conn)?.claude = nil
        default:
            break
        }
    }

    private func connectionClosed(_ conn: DeckConnection) {
        if let t = tab(for: conn) { t.claude = nil; t.connection = nil }
        if pick?.connection === conn { finishPick() }
        pickQueue.removeAll { $0.connection === conn }
    }

    func answerPick(_ req: PickRequest, account: String?, remember: Bool) {
        if let account {
            req.connection.send(["account": account])
            if remember, config.account(for: req.cwd) != account {
                Task { try? await DeckCLI.map(account, dir: req.cwd); await refresh() }
            }
        } else {
            req.connection.send(["cancel": true])
        }
        finishPick()
        if let t = tabs.first(where: { $0.id == req.tabID }) {
            DispatchQueue.main.async { t.view.window?.makeFirstResponder(t.view) }
        }
    }

    private func finishPick() {
        pick = nil
        if !pickQueue.isEmpty {
            let next = pickQueue.removeFirst()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { self.pick = next }
        }
    }

    /// Restart claude in `tab` on another account, resuming the same conversation.
    func move(_ tab: TerminalTab, to account: String, reason: String = "Moved") {
        guard let run = tab.claude, let conn = tab.connection, run.account != account else { return }
        let claimed = tabs.filter { $0.id != tab.id }.compactMap { $0.claude?.sessionID }
        conn.send(["type": "switch", "account": account, "claimed": claimed])
        tab.lastSwitch = Date()
        flash("\(reason): \(run.account) → \(account)")
        if reason != "Moved" {
            Task { await DeckCLI.event("auto_switch", "\(run.account) → \(account)", account: account) }
        }
    }

    private func autoSwitchTick() {
        for tab in tabs {
            guard let run = tab.claude, let acc = account(run.account), acc.over else {
                tab.warned = false
                continue
            }
            if !config.auto_switch {
                if !tab.warned {
                    tab.warned = true
                    let msg = "\(acc.name) is near its usage limit."
                    flash(msg + " Use the account menu to move the session.")
                    notify("Usage limit", msg)
                    Task { await DeckCLI.event("limit", msg, account: acc.name) }
                }
                continue
            }
            if tab.idleFor < Double(config.idle_secs) { continue }
            if let last = tab.lastSwitch, Date().timeIntervalSince(last) < Double(config.cooldown_secs) { continue }
            if let target = best(excluding: acc.name) {
                move(tab, to: target.name, reason: "Auto-switched")
                notify("Session moved", "\(acc.name) was near its limit, now on \(target.name).")
            } else if !tab.warned {
                tab.warned = true
                flash("Every account is near its limit.")
                notify("Usage limit", "Every account is near its limit.")
            }
        }
    }
}
