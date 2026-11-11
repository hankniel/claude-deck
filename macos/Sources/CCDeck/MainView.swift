import SwiftUI

typealias Theme = DeckTheme

struct MainView: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        NavigationSplitView(columnVisibility: $state.columns) {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 220, ideal: 256, max: 340)
        } detail: {
            DetailView()
        }
        .sheet(item: $state.pick) { req in
            AccountPicker(request: req).environmentObject(state)
        }
        .overlay(alignment: .bottom) { Toast() }
        .frame(minWidth: 880, minHeight: 540)
    }
}

// MARK: - Shared pieces

/// Activity-style concentric rings: outer = 5-hour window, inner = 7-day window.
struct UsageRings: View {
    let five: Double?
    let week: Double?
    var threshold: Double = 90
    var weekly: Double = 95
    var size: CGFloat = 26
    var line: CGFloat = 3.5

    var body: some View {
        ZStack {
            ring(five, line: threshold, inset: 0)
            ring(week, line: weekly, inset: line + 1.5)
        }
        .frame(width: size, height: size)
    }

    private func ring(_ pct: Double?, line limit: Double, inset: CGFloat) -> some View {
        let p = min(max(pct ?? 0, 0), 100) / 100
        let color = pct == nil ? Color.secondary : Theme.usageColor(pct ?? 0, line: limit)
        return ZStack {
            Circle().stroke(color.opacity(0.18), lineWidth: line)
            Circle().trim(from: 0, to: p)
                .stroke(color, style: StrokeStyle(lineWidth: line, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .padding(inset + line / 2)
    }
}

struct PlanBadge: View {
    let text: String
    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 5).padding(.vertical, 1)
            .background(.quaternary, in: Capsule())
    }
}

struct ThinBar: View {
    let pct: Double
    let color: Color
    var height: CGFloat = 4

    var body: some View {
        GeometryReader { g in
            ZStack(alignment: .leading) {
                Capsule().fill(.quaternary)
                Capsule().fill(color).frame(width: max(height, g.size.width * min(pct, 100) / 100))
            }
        }
        .frame(height: height)
    }
}

/// Two labelled bars (5h / 7d), used in the menu bar panel.
struct UsageBars: View {
    let account: AccountInfo
    let threshold: Double
    let weekly: Double

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            if let w = account.five { row("5h", w, threshold) }
            if let w = account.week { row("7d", w, weekly) }
        }
    }

    private func row(_ label: String, _ w: UsageWindow, _ th: Double) -> some View {
        HStack(spacing: 8) {
            Text(label).font(.caption2.monospacedDigit()).foregroundStyle(.secondary).frame(width: 16, alignment: .leading)
            ThinBar(pct: w.pct, color: Theme.usageColor(w.pct, line: th))
            Text("\(Int(w.pct.rounded()))%").font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
                .frame(width: 30, alignment: .trailing)
        }
        .help("\(label == "5h" ? "5-hour" : "7-day") window · resets in \(w.resets_in)")
    }
}

func usageSummary(_ a: AccountInfo) -> String {
    guard a.state == "ok" else { return a.stateLabel }
    var parts: [String] = []
    if let f = a.five { parts.append("5h \(Int(f.pct.rounded()))%") }
    if let w = a.week { parts.append("7d \(Int(w.pct.rounded()))%") }
    if let r = a.five?.resets_in, !r.isEmpty { parts.append("↻ \(r)") }
    return parts.isEmpty ? "No limits reported" : parts.joined(separator: " · ")
}

struct Toast: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        Group {
            if let n = state.notice {
                Label(n, systemImage: "info.circle")
                    .font(.callout)
                    .padding(.horizontal, 16).padding(.vertical, 10)
                    .glassEffect(.regular, in: .capsule)
                    .padding(.bottom, 22)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.spring(duration: 0.35), value: state.notice)
    }
}

// MARK: - Sidebar

struct SidebarView: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        List(selection: $state.selection) {
            Section("Sessions") {
                ForEach(state.tabs) { tab in
                    TabRow(tab: tab, canvas: state.canvas)
                        .tag(SidebarItem.tab(tab.id))
                        .contextMenu {
                            Button("Start Claude Here", systemImage: "sparkle") {
                                state.selection = .tab(tab.id); tab.type("claude\r")
                            }
                            Button("New Terminal in This Folder", systemImage: "plus.rectangle.on.folder") {
                                state.newTab(cwd: tab.cwd)
                            }
                            Divider()
                            Button("Close Terminal", systemImage: "xmark") { state.close(tab) }
                        }
                }
            }

            Section("Insights") {
                InsightsRows(canvas: state.canvas)
            }

            Section {
                ForEach(state.accounts) { acc in
                    AccountRow(account: acc, inUse: state.tabs.contains { $0.claude?.account == acc.name })
                        .selectionDisabled()
                }
            } header: {
                HStack {
                    Text("Accounts")
                    Spacer()
                    if state.refreshing { ProgressView().controlSize(.mini) }
                    Menu {
                        Toggle("Auto-switch near limits", isOn: Binding(
                            get: { state.config.auto_switch },
                            set: { state.setConfig("auto_switch", $0 ? "true" : "false") }))
                        Button("Refresh usage", systemImage: "arrow.clockwise") { Task { await state.refresh() } }
                        Divider()
                        SettingsLink { Label("Manage accounts…", systemImage: "person.2") }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                    .menuStyle(.button)
                    .buttonStyle(.plain)
                    .menuIndicator(.hidden)
                    .fixedSize()
                    .foregroundStyle(.secondary)
                }
            }
        }
        .listStyle(.sidebar)
        .tint(Theme.accent)
    }
}

struct InsightsRows: View {
    @ObservedObject var canvas: CanvasModel

    var body: some View {
        Label("Canvas", systemImage: "rectangle.on.rectangle.angled")
            .badge(canvas.unseen)
            .tag(SidebarItem.canvas)
            .listItemTint(Theme.accent)
        Label("Board", systemImage: "checklist")
            .badge(canvas.waiting.count)
            .tag(SidebarItem.board)
            .listItemTint(Theme.accent)
        Label("Jira", systemImage: "square.stack.3d.up")
            .tag(SidebarItem.jira)
            .listItemTint(Theme.accent)
        Label("Dashboard", systemImage: "chart.bar.xaxis")
            .tag(SidebarItem.dashboard)
            .listItemTint(Theme.accent)
    }
}

struct TabRow: View {
    @ObservedObject var tab: TerminalTab
    @ObservedObject var canvas: CanvasModel

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 1) {
                Text(tab.displayTitle).lineLimit(1)
                let agent = tab.claude != nil ? canvas.agent(forPane: tab.id.uuidString) : nil
                Text(agent.map { a in a.stateText.isEmpty ? tab.subtitle : "\(tab.subtitle) · \(a.stateText)" } ?? tab.subtitle)
                    .font(.caption).foregroundStyle(agent?.needsYou == true ? Color.orange : .secondary).lineLimit(1)
            }
        } icon: {
            Image(systemName: tab.claude != nil ? "sparkle" : "terminal")
                .foregroundStyle(tab.claude != nil ? Theme.accent : .secondary)
        }
        .padding(.vertical, 2)
    }
}

struct AccountRow: View {
    let account: AccountInfo
    var inUse = false
    @EnvironmentObject var state: AppState

    var body: some View {
        HStack(spacing: 10) {
            UsageRings(five: account.five?.pct, week: account.week?.pct,
                       threshold: state.config.threshold, weekly: state.config.weekly_threshold)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(account.name).fontWeight(.medium)
                    if let p = account.planLabel { PlanBadge(text: p) }
                    if inUse {
                        Image(systemName: "sparkle").font(.caption2).foregroundStyle(Theme.accent)
                            .help("A Claude session is running on this account")
                    }
                }
                Text(usageSummary(account))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(account.over ? .red : .secondary)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 3)
        .help(account.email ?? account.name)
    }
}

// MARK: - Detail

struct DetailView: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        switch state.selection {
        case .dashboard:
            DashboardView()
        case .canvas:
            CanvasPageView(model: state.canvas)
        case .board:
            BoardView(model: state.canvas)
        case .jira:
            JiraView(canvas: state.canvas)
        default:
            if let tab = state.activeTab {
                TerminalPane(tab: tab)
            } else {
                ContentUnavailableView {
                    Label("No terminal open", systemImage: "apple.terminal")
                } description: {
                    Text("Open a terminal, then type `claude` to pick the account for that session.")
                } actions: {
                    Button("New Terminal") { state.newTab() }
                        .buttonStyle(.glassProminent)
                        .tint(Theme.accent)
                }
            }
        }
    }
}

struct TerminalPane: View {
    @ObservedObject var tab: TerminalTab
    @EnvironmentObject var state: AppState

    var body: some View {
        VStack(spacing: 0) {
            if let run = tab.claude, let acc = state.account(run.account), acc.over, !state.config.auto_switch,
               let target = state.best(excluding: acc.name) {
                HStack(spacing: 10) {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                    Text("**\(acc.name)** is near its usage limit.")
                    Spacer()
                    Button("Move to \(target.name)") { state.move(tab, to: target.name) }
                        .buttonStyle(.glassProminent)
                        .tint(Theme.accent)
                        .controlSize(.small)
                }
                .font(.callout)
                .padding(.horizontal, 16).padding(.vertical, 8)
                .background(.orange.opacity(0.08))
            }
            TerminalHost(tab: tab)
                .id(tab.id)
        }
        .background(Theme.terminalBackground.ignoresSafeArea())
        .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
        .navigationTitle(tab.displayTitle)
        .navigationSubtitle(DeckFormat.shortPath(tab.claude?.cwd ?? tab.cwd))
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                if let run = tab.claude {
                    AccountMenu(tab: tab, run: run)
                } else {
                    Button { tab.type("claude\r") } label: {
                        Label("Start Claude", systemImage: "sparkle")
                    }
                    .labelStyle(.titleAndIcon)
                    .help("Run claude in this terminal (⇧⌘K)")
                }
            }
            ToolbarSpacer(.fixed, placement: .primaryAction)
            ToolbarItem(placement: .primaryAction) {
                Button { state.newTab() } label: { Label("New Terminal", systemImage: "plus") }
                    .help("New terminal (⌘T)")
            }
        }
    }
}

struct AccountMenu: View {
    @ObservedObject var tab: TerminalTab
    let run: ClaudeRun
    @EnvironmentObject var state: AppState

    var body: some View {
        Menu {
            Section("Run this conversation on") {
                ForEach(state.accounts) { a in
                    Button {
                        state.move(tab, to: a.name)
                    } label: {
                        Text(a.name)
                        Text(usageSummary(a))
                    }
                    .disabled(a.name == run.account || a.state == "logged_out")
                }
            }
            Section {
                Text("Moving restarts claude and resumes this chat.")
            }
        } label: {
            let pct = state.account(run.account)?.five?.pct
            Label(pct.map { "\(run.account) · \(Int($0))%" } ?? run.account, systemImage: "person.crop.circle")
        }
        .labelStyle(.titleAndIcon)
        .help("Account for this Claude session")
    }
}

// MARK: - Account picker (shown when `claude` is typed)

struct AccountPicker: View {
    let request: PickRequest
    @EnvironmentObject var state: AppState
    @State private var selected: String?
    @State private var remember = false
    @FocusState private var focused: Bool

    private var mapped: String? { state.config.account(for: request.cwd) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "sparkle")
                    .font(.title2)
                    .foregroundStyle(Theme.accent)
                    .frame(width: 40, height: 40)
                    .glassEffect(.regular.tint(Theme.accent.opacity(0.25)), in: .circle)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Run Claude as…").font(.title2.weight(.semibold))
                    Label(DeckFormat.shortPath(request.cwd), systemImage: "folder")
                        .font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                }
                Spacer()
                Button { Task { await state.refresh() } } label: { Image(systemName: "arrow.clockwise") }
                    .buttonStyle(.borderless)
                    .help("Refresh usage")
            }

            VStack(spacing: 8) {
                ForEach(Array(state.accounts.enumerated()), id: \.element.id) { i, acc in
                    PickerRow(index: i + 1, account: acc, mapped: acc.name == mapped, selected: acc.name == selected)
                        .contentShape(.rect(cornerRadius: 14))
                        .onTapGesture(count: 2) { choose(acc.name) }
                        .onTapGesture { selected = acc.name }
                }
            }
            .focusable()
            .focusEffectDisabled()
            .focused($focused)
            .onKeyPress(.upArrow) { move(-1); return .handled }
            .onKeyPress(.downArrow) { move(1); return .handled }
            .onKeyPress(characters: .decimalDigits) { press in
                guard let n = Int(press.characters), n >= 1, n <= state.accounts.count else { return .ignored }
                choose(state.accounts[n - 1].name)
                return .handled
            }

            Toggle("Always use this account in this folder", isOn: $remember)

            HStack {
                Text("↑↓ or 1–9 to choose").font(.caption).foregroundStyle(.tertiary)
                Spacer()
                Button("Cancel", role: .cancel) { state.answerPick(request, account: nil, remember: false) }
                    .keyboardShortcut(.cancelAction)
                    .tint(.primary)
                Button("Start Claude") { if let s = selected { choose(s) } }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.glassProminent)
                    .tint(Theme.accent)
                    .disabled(selected == nil)
            }
            .controlSize(.large)
        }
        .padding(22)
        .frame(width: 500)
        .onAppear {
            selected = mapped ?? state.best(excluding: nil)?.name ?? state.accounts.first?.name
            remember = mapped != nil
            focused = true
        }
    }

    private func move(_ d: Int) {
        let names = state.accounts.map(\.name)
        guard !names.isEmpty else { return }
        let i = selected.flatMap { names.firstIndex(of: $0) } ?? 0
        selected = names[min(max(i + d, 0), names.count - 1)]
    }

    private func choose(_ name: String) {
        state.answerPick(request, account: name, remember: remember)
    }
}

struct PickerRow: View {
    let index: Int
    let account: AccountInfo
    let mapped: Bool
    var selected = false
    @EnvironmentObject var state: AppState

    var body: some View {
        HStack(spacing: 12) {
            UsageRings(five: account.five?.pct, week: account.week?.pct,
                       threshold: state.config.threshold, weekly: state.config.weekly_threshold,
                       size: 34, line: 4)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(account.name).font(.headline)
                    if let p = account.planLabel { PlanBadge(text: p) }
                    if mapped {
                        Text("Folder default").font(.caption2.weight(.semibold)).foregroundStyle(Theme.accent)
                    }
                }
                Text(account.email ?? "")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Text(account.state == "logged_out" ? "Not logged in — you can log in inside the session" : usageSummary(account))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(account.over ? .red : .secondary)
            }
            Spacer()
            Text("\(index)")
                .font(.callout.monospacedDigit())
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 14).padding(.vertical, 12)
        .background(selected ? Theme.accent.opacity(0.12) : Color.primary.opacity(0.04),
                    in: .rect(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous)
            .strokeBorder(selected ? Theme.accent.opacity(0.8) : Color.primary.opacity(0.08), lineWidth: selected ? 1.5 : 1))
        .opacity(account.state == "ok" && account.over && !selected ? 0.7 : 1)
        .animation(.snappy(duration: 0.15), value: selected)
    }
}
