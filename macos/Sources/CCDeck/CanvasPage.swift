import AppKit
import SwiftUI

enum CanvasTab: Hashable, Identifiable {
    case screen(session: String, name: String)
    case file(path: String, line: Int?)
    case git(session: String)

    var id: String {
        switch self {
        case .screen(let s, let n): return "s:\(s)/\(n)"
        case .file(let p, _): return "f:\(p)"
        case .git(let s): return "g:\(s)"
        }
    }
}

/// Canvas sessions, objectives and open tabs, refreshed by polling the CLI.
@MainActor
final class CanvasModel: ObservableObject {
    @Published var sessions: [CanvasSession] = []
    @Published var objectives: [Objective] = []
    @Published var tabs: [CanvasTab] = []
    @Published var selected: CanvasTab?
    @Published var expanded: Set<String> = []
    @Published var showEnded = false
    @Published var unseen = 0

    var toast: (String) -> Void = { _ in }
    /// (session, previous state) when a session's state changes.
    var onState: (CanvasSession, String?) -> Void = { _, _ in }
    var isVisible: () -> Bool = { false }

    private var lastData = Data()
    private var lastObjectives = Data()
    private var seen: [String: Int64] = [:]
    private var states: [String: String] = [:]
    private var loaded = false
    private var timer: Timer?
    private var polling = false

    func start() {
        guard timer == nil else { return }
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            Task { @MainActor in await self?.poll() }
        }
        Task { await poll() }
    }

    func session(_ id: String) -> CanvasSession? { sessions.first { $0.id == id } }

    /// The canvas session of the agent running in a CC Deck tab.
    func agent(forPane pane: String) -> CanvasSession? {
        sessions.filter { $0.meta.pane == pane && $0.state.state != "ended" }.max { $0.updated < $1.updated }
    }

    var waiting: [CanvasSession] { sessions.filter(\.needsYou) }

    func poll() async {
        guard !polling else { return }
        polling = true
        defer { polling = false }
        guard let result = try? await CanvasCLI.sessions() else { return }
        let (data, list) = result
        if let od = try? await DeckCLI.run(["canvas", "objectives", "--json"]), od != lastObjectives {
            lastObjectives = od
            objectives = (try? JSONDecoder().decode([Objective].self, from: od)) ?? objectives
        }
        guard data != lastData else { return }
        lastData = data
        apply(list)
    }

    func forceRefresh() {
        lastData = Data()
        lastObjectives = Data()
        Task { await poll() }
    }

    private func apply(_ list: [CanvasSession]) {
        let first = !loaded
        if first {
            for s in list.filter({ !$0.items.isEmpty }).prefix(4) { expanded.insert(s.id) }
        }
        var changed: [(String, String)] = []
        for s in list {
            for it in s.items {
                let key = "\(s.id)/\(it.name)"
                let prev = seen.updateValue(it.mtime, forKey: key)
                if !first, prev == nil || prev! < it.mtime { changed.append((s.id, it.name)) }
            }
            let prevState = states.updateValue(s.state.state, forKey: s.id)
            if !first, prevState != s.state.state { onState(s, prevState) }
        }
        sessions = list
        loaded = true
        // tabs whose screens were deleted
        tabs.removeAll { tab in
            if case .screen(let sid, let name) = tab { return !(list.first { $0.id == sid }?.items.contains { $0.name == name } ?? false) }
            return false
        }
        if let sel = selected, !tabs.contains(sel) { selected = tabs.last }
        for (sid, name) in changed {
            guard let s = list.first(where: { $0.id == sid }), s.state.state != "ended" else { continue }
            let tab = CanvasTab.screen(session: sid, name: name)
            expanded.insert(sid)
            if !tabs.contains(tab) { tabs.append(tab) }
            if isVisible() || selected == nil { selected = tab }
        }
        if !changed.isEmpty && !isVisible() {
            unseen += changed.count
            if let first = changed.first { toast("New on the canvas: \(first.1)") }
        }
    }

    func open(_ tab: CanvasTab) {
        if case .file(let p, _) = tab {
            tabs.removeAll { if case .file(let q, _) = $0 { return q == p }; return false }
        }
        if !tabs.contains(tab) { tabs.append(tab) }
        selected = tab
    }

    func close(_ tab: CanvasTab) {
        guard let i = tabs.firstIndex(of: tab) else { return }
        tabs.remove(at: i)
        if selected == tab { selected = tabs.isEmpty ? nil : tabs[min(i, tabs.count - 1)] }
    }

    func closeSelected() -> Bool {
        guard let s = selected else { return false }
        close(s)
        return true
    }

    func deleteSession(_ s: CanvasSession) {
        tabs.removeAll { if case .screen(let sid, _) = $0 { return sid == s.id }; if case .git(let sid) = $0 { return sid == s.id }; return false }
        try? FileManager.default.removeItem(atPath: s.dir)
        forceRefresh()
    }

    func deleteScreen(_ s: CanvasSession, _ name: String) {
        close(.screen(session: s.id, name: name))
        try? FileManager.default.removeItem(atPath: s.path(name))
        forceRefresh()
    }

    func title(_ tab: CanvasTab) -> String {
        switch tab {
        case .screen(_, let n): return n
        case .file(let p, _): return (p as NSString).lastPathComponent
        case .git(let s): return "Changes · \(session(s)?.meta.project ?? "")"
        }
    }

    func icon(_ tab: CanvasTab) -> String {
        switch tab {
        case .screen(let sid, let n): return CanvasFormat.icon(session(sid)?.items.first { $0.name == n }?.kind ?? "")
        case .file: return "doc.text"
        case .git: return "arrow.triangle.pull"
        }
    }

    func visibleSessions() -> [CanvasSession] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        return sessions.filter { s in
            if showEnded { return true }
            if s.state.state == "ended" { return !s.items.isEmpty && now - s.updated < 24 * 3600 * 1000 }
            return !s.items.isEmpty || !s.state.state.isEmpty
        }
    }
}

func stateColor(_ s: String) -> Color {
    switch s {
    case "running": return .green
    case "waiting": return .orange
    case "idle": return .gray
    case "ended": return .gray.opacity(0.5)
    default: return .gray.opacity(0.35)
    }
}

// MARK: - page

struct CanvasPageView: View {
    @ObservedObject var model: CanvasModel
    @EnvironmentObject var state: AppState

    var body: some View {
        HSplitView {
            explorer.frame(minWidth: 230, idealWidth: 280, maxWidth: 420)
            VStack(spacing: 0) {
                if model.tabs.isEmpty {
                    ContentUnavailableView {
                        Label("Canvas", systemImage: "rectangle.on.rectangle.angled")
                    } description: {
                        Text("Screens your agents draw show up here: flowcharts (.mmd), data lineage (.lineage), database diagrams (.dbml), review findings (.rv), row counts (.rows), plans (.plan) and documents to comment on (.doc).\n\nAgents learn the formats from the canvas skill; ask one to draw something.")
                    }
                } else {
                    tabStrip
                    Divider()
                    if let sel = model.selected {
                        CanvasTabContent(tab: sel, model: model).id(sel.id)
                    }
                }
            }
            .frame(minWidth: 400)
        }
        .navigationTitle("Canvas")
        .navigationSubtitle(model.selected.map(model.title) ?? "")
        .onAppear { model.unseen = 0 }
    }

    private var explorer: some View {
        List {
            Section {
                ForEach(model.visibleSessions()) { s in
                    sessionRow(s)
                    if model.expanded.contains(s.id) {
                        ForEach(s.items) { it in
                            Button { model.open(.screen(session: s.id, name: it.name)) } label: {
                                HStack(spacing: 8) {
                                    Image(systemName: CanvasFormat.icon(it.kind)).foregroundStyle(.secondary).frame(width: 16)
                                    Text(it.name).lineLimit(1).truncationMode(.middle)
                                    Spacer()
                                    Text(CanvasFormat.ago(it.mtime)).font(.caption).foregroundStyle(.tertiary)
                                }
                                .padding(.leading, 16)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .listRowBackground(model.selected == .screen(session: s.id, name: it.name) ? DeckTheme.accent.opacity(0.15) : Color.clear)
                            .contextMenu {
                                Button("Delete Screen", role: .destructive) { model.deleteScreen(s, it.name) }
                            }
                        }
                    }
                }
            } header: {
                HStack {
                    Text("Agent sessions")
                    Spacer()
                    Toggle(isOn: $model.showEnded) { Image(systemName: "clock.arrow.circlepath") }
                        .toggleStyle(.button).buttonStyle(.borderless).help("Show ended sessions")
                }
            }
        }
        .listStyle(.sidebar)
    }

    private func sessionRow(_ s: CanvasSession) -> some View {
        Button {
            if model.expanded.contains(s.id) { model.expanded.remove(s.id) } else { model.expanded.insert(s.id) }
        } label: {
            HStack(spacing: 8) {
                Circle().fill(stateColor(s.state.state)).frame(width: 8, height: 8).help(s.stateText)
                VStack(alignment: .leading, spacing: 1) {
                    Text(s.label).lineLimit(1)
                    Text([s.meta.project, s.meta.account, s.stateText, CanvasFormat.ago(s.updated)].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.caption).foregroundStyle(s.needsYou ? Color.orange : .secondary).lineLimit(1)
                }
                Spacer()
                Text("\(s.items.count) \(model.expanded.contains(s.id) ? "▾" : "▸")").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("\(s.meta.cwd)\n\(s.meta.id)")
        .contextMenu {
            if state.tabs.contains(where: { $0.id.uuidString == s.meta.pane }) {
                Button("Go to Terminal") { state.goToPane(s.meta.pane) }
            }
            if !s.meta.cwd.isEmpty { Button("Show Changes") { model.open(.git(session: s.id)) } }
            Divider()
            Button("Delete Session", role: .destructive) { model.deleteSession(s) }
        }
    }

    private var tabStrip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 4) {
                ForEach(model.tabs) { tab in
                    let on = model.selected == tab
                    HStack(spacing: 6) {
                        Image(systemName: model.icon(tab)).font(.caption)
                        Text(model.title(tab)).lineLimit(1)
                        Button { model.close(tab) } label: { Image(systemName: "xmark").font(.caption2) }
                            .buttonStyle(.borderless).opacity(on ? 1 : 0.5)
                    }
                    .padding(.horizontal, 10).padding(.vertical, 5)
                    .background(on ? DeckTheme.accent.opacity(0.18) : Color.clear, in: Capsule())
                    .contentShape(Capsule())
                    .onTapGesture { model.selected = tab }
                }
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
        }
    }
}

struct CanvasTabContent: View {
    let tab: CanvasTab
    @ObservedObject var model: CanvasModel
    @EnvironmentObject var state: AppState

    private func open(cwd: String) -> OpenHandler {
        { p, line in
            if p.hasPrefix("http://") || p.hasPrefix("https://") {
                if let u = URL(string: p) { NSWorkspace.shared.open(u) }
            } else {
                model.open(.file(path: resolvePath(p, cwd: cwd), line: line))
            }
        }
    }

    var body: some View {
        switch tab {
        case .file(let path, let line):
            FileScreen(path: path, line: line, open: open(cwd: (path as NSString).deletingLastPathComponent))
        case .git(let sid):
            if let s = model.session(sid) { GitChangesView(session: s) } else { Text("Session gone").foregroundStyle(.secondary) }
        case .screen(let sid, let name):
            if let s = model.session(sid), let item = s.items.first(where: { $0.name == name }) {
                let ctx = ScreenContext(session: s, name: name, open: open(cwd: s.meta.cwd), toast: { t in state.flash(t) })
                ScreenHost(ctx: ctx, kind: item.kind)
            } else {
                Text("This screen is gone").foregroundStyle(.secondary)
            }
        }
    }
}

struct ScreenHost: View {
    let ctx: ScreenContext
    let kind: String
    @State var flowKind: SceneKind?

    var body: some View {
        switch kind {
        case "flow":
            Group {
                switch flowKind?.kind {
                case "flow": FlowDiagramView(file: ctx.path, version: ctx.version, open: ctx.open)
                case "sequence": Parsed<SeqScene, SequenceDiagramView>(file: ctx.path, version: ctx.version) { SequenceDiagramView(scene: $0) }
                case "source":
                    SourceScreen(path: ctx.path, note: flowKind?.error.map { "This diagram can't be drawn: \($0). CC Deck draws flowcharts and sequence diagrams natively." }
                                 ?? "Shown as source.")
                default: ProgressView()
                }
            }
            .task(id: ctx.version) { flowKind = try? await CanvasCLI.kind(of: ctx.path) }
        case "lineage": LineageDiagramView(file: ctx.path, version: ctx.version)
        case "erd": ErdDiagramView(file: ctx.path, version: ctx.version)
        case "review": Parsed<ReviewDoc, ReviewScreen>(file: ctx.path, version: ctx.version) { ReviewScreen(ctx: ctx, doc: $0) }
        case "rows": Parsed<RowsDoc, RowsScreen>(file: ctx.path, version: ctx.version) { RowsScreen(doc: $0, name: ctx.name) }
        case "doc":
            Parsed<PathDoc, DocScreen>(file: ctx.path, version: ctx.version) { DocScreen(ctx: ctx, path: resolvePath($0.path, cwd: ctx.session.meta.cwd)) }
        case "open":
            Parsed<PathDoc, FileScreen>(file: ctx.path, version: ctx.version) { FileScreen(path: resolvePath($0.path, cwd: ctx.session.meta.cwd), line: $0.line, open: ctx.open) }
        case "markdown": Parsed<MarkdownDoc, MarkdownScreen>(file: ctx.path, version: ctx.version) { MarkdownScreen(blocks: $0.blocks) }
        case "plan": Parsed<PlanDoc, PlanScreen>(file: ctx.path, version: ctx.version) { PlanScreen(ctx: ctx, doc: $0) }
        case "html": SourceScreen(path: ctx.path, note: "HTML screens open in your browser.", browser: true)
        default: SourceScreen(path: ctx.path, note: "Unknown screen type.")
        }
    }
}
