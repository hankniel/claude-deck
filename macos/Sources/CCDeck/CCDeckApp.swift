import AppKit
import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    var onTerminate: (() -> Void)?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }

    func applicationWillTerminate(_ notification: Notification) { onTerminate?() }
}

@main
struct CCDeckApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var state = AppState()
    @Environment(\.openWindow) private var openWindow

    var body: some Scene {
        Window("CC Deck", id: "main") {
            MainView()
                .environmentObject(state)
                .tint(DeckTheme.accent)
                .onAppear { let s = state; delegate.onTerminate = { s.shutdown() } }
        }
        .windowToolbarStyle(.unified)
        .defaultSize(width: 1240, height: 800)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("New Terminal") { state.newTab() }.keyboardShortcut("t")
                Button("New Terminal in Home Folder") { state.newTab(cwd: NSHomeDirectory()) }
                    .keyboardShortcut("t", modifiers: [.command, .option])
            }
            CommandGroup(replacing: .saveItem) {
                Button("Close Terminal") { state.closeActive() }.keyboardShortcut("w")
            }
            CommandMenu("Claude") {
                Button("Start Claude in This Terminal") { state.startClaudeInActiveTab() }
                    .keyboardShortcut("k", modifiers: [.command, .shift])
                Menu("Move Session To") {
                    ForEach(state.accounts) { acc in
                        Button(acc.name) { if let t = state.activeTab { state.move(t, to: acc.name) } }
                            .disabled(state.activeTab?.claude == nil || state.activeTab?.claude?.account == acc.name)
                    }
                }
                Divider()
                Toggle("Auto-switch Near Limits", isOn: Binding(get: { state.config.auto_switch },
                                                                 set: { state.setConfig("auto_switch", $0 ? "true" : "false") }))
                Button("Refresh Usage") { Task { await state.refresh() } }.keyboardShortcut("r", modifiers: [.command, .shift])
                Divider()
                Button("Dashboard") { state.selection = .dashboard }.keyboardShortcut("d", modifiers: [.command, .shift])
                Button("Canvas") { state.selection = .canvas }.keyboardShortcut("e", modifiers: [.command, .shift])
                Button("Board") { state.selection = .board }.keyboardShortcut("o", modifiers: [.command, .shift])
                Button("Jira") { state.selection = .jira }.keyboardShortcut("j", modifiers: [.command, .shift])
            }
            CommandGroup(after: .windowArrangement) {
                Divider()
                Button("Next Terminal") { state.cycle(1) }.keyboardShortcut("]", modifiers: [.command, .shift])
                Button("Previous Terminal") { state.cycle(-1) }.keyboardShortcut("[", modifiers: [.command, .shift])
                ForEach(1..<10) { i in
                    Button("Terminal \(i)") { state.select(index: i - 1) }
                        .keyboardShortcut(KeyEquivalent(Character("\(i)")), modifiers: .command)
                }
            }
        }

        Settings {
            SettingsView()
                .environmentObject(state)
                .tint(DeckTheme.accent)
        }

        MenuBarExtra {
            MenuBarView()
                .environmentObject(state)
        } label: {
            Image(systemName: menuBarSymbol)
        }
        .menuBarExtraStyle(.window)
    }

    private var menuBarSymbol: String {
        let worst = state.accounts.compactMap { $0.five?.pct }.max() ?? 0
        switch worst {
        case ..<50: return "gauge.with.dots.needle.33percent"
        case ..<85: return "gauge.with.dots.needle.67percent"
        default: return "gauge.with.dots.needle.100percent"
        }
    }
}

struct MenuBarView: View {
    @EnvironmentObject var state: AppState
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("CC Deck").font(.headline)
                Spacer()
                if let t = state.lastRefresh {
                    Text(t, format: .relative(presentation: .named)).font(.caption).foregroundStyle(.secondary)
                }
                Button { Task { await state.refresh() } } label: { Image(systemName: "arrow.clockwise") }
                    .buttonStyle(.borderless)
            }
            ForEach(state.accounts) { acc in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(acc.name).fontWeight(.medium)
                        if let p = acc.planLabel { PlanBadge(text: p) }
                        Spacer()
                        if let r = acc.five?.resets_in, !r.isEmpty {
                            Text("↻ \(r)").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    if acc.state == "ok" {
                        UsageBars(account: acc, threshold: state.config.threshold, weekly: state.config.weekly_threshold)
                    } else {
                        Text(acc.stateLabel).font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            Divider()
            HStack {
                Button("Open CC Deck") {
                    openWindow(id: "main")
                    NSApp.activate(ignoringOtherApps: true)
                }
                Spacer()
                Button("Dashboard") {
                    state.selection = .dashboard
                    openWindow(id: "main")
                    NSApp.activate(ignoringOtherApps: true)
                }
                Button("Quit") { NSApp.terminate(nil) }
            }
            .controlSize(.small)
        }
        .padding(14)
        .frame(width: 300)
        .tint(DeckTheme.accent)
    }
}
