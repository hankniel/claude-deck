import AppKit
import SwiftUI

// MARK: - Board

struct BoardView: View {
    @ObservedObject var model: CanvasModel
    @EnvironmentObject var state: AppState
    @State var adding: [String: String] = [:]
    @State var noting: String?
    @State var noteText = ""

    private func glyph(_ s: String) -> String {
        switch s { case "doing": return "circle.lefthalf.filled"; case "done": return "checkmark.circle.fill"; case "blocked": return "nosign"; default: return "circle" }
    }

    private func tint(_ s: String) -> Color {
        switch s { case "doing": return DeckTheme.accent; case "done": return .green; case "blocked": return .red; default: return .secondary }
    }

    private func next(_ s: String) -> String { s == "todo" ? "doing" : s == "doing" ? "done" : "todo" }

    private func apply(_ slug: String, _ op: [String: Any]) {
        Task {
            do { state.flash(try await CanvasCLI.edit(slug, op) + " — agents on it hear this with their next prompt") }
            catch { state.flash(error.localizedDescription) }
            model.forceRefresh()
        }
    }

    private func chip(_ s: CanvasSession) -> some View {
        Button { goto(s) } label: {
            HStack(spacing: 8) {
                Circle().fill(stateColor(s.state.state)).frame(width: 8, height: 8)
                VStack(alignment: .leading, spacing: 0) {
                    Text(s.label).lineLimit(1)
                    Text([s.meta.agent, s.meta.project, s.meta.account, s.stateText].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.caption).foregroundStyle(s.needsYou ? Color.orange : .secondary).lineLimit(1)
                }
            }
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background(.background.secondary, in: .rect(cornerRadius: 10))
        }
        .buttonStyle(.plain)
        .help("Go to this session")
    }

    private func goto(_ s: CanvasSession) {
        if state.tabs.contains(where: { $0.id.uuidString == s.meta.pane }) {
            state.goToPane(s.meta.pane)
        } else {
            state.selection = .canvas
            if let it = s.items.first { model.open(.screen(session: s.id, name: it.name)) }
        }
    }

    private func card(_ o: Objective) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(o.title).font(.title3.weight(.semibold))
                    let blocked = o.tasks.filter { $0.state == "blocked" }.count
                    Text("\(o.done)/\(o.tasks.count) done · \(o.slug)\(blocked > 0 ? " · \(blocked) blocked" : "")")
                        .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                }
                Spacer()
                Picker("", selection: Binding(get: { o.status }, set: { apply(o.slug, ["op": "status", "status": $0]) })) {
                    Text("active").tag("active"); Text("paused").tag("paused"); Text("done").tag("done")
                }
                .labelsHidden().fixedSize()
            }
            ProgressView(value: o.tasks.isEmpty ? 0 : Double(o.done) / Double(o.tasks.count)).tint(DeckTheme.accent)
            ForEach(o.tasks) { t in
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 8) {
                        Button { apply(o.slug, ["op": "state", "line": t.line, "text": t.text, "state": next(t.state)]) } label: {
                            Image(systemName: glyph(t.state)).foregroundStyle(tint(t.state))
                        }
                        .buttonStyle(.borderless).help("\(t.state) — click for \(next(t.state))")
                        Text(t.text).foregroundStyle(t.state == "done" ? .secondary : .primary)
                        Spacer()
                        Menu {
                            ForEach(["todo", "doing", "done", "blocked"], id: \.self) { s in
                                Button(s) { apply(o.slug, ["op": "state", "line": t.line, "text": t.text, "state": s]) }.disabled(s == t.state)
                            }
                            Divider()
                            Button("Add a note…") { noting = "\(o.slug)#\(t.line)"; noteText = "" }
                        } label: { Image(systemName: "ellipsis") }
                        .menuStyle(.button).buttonStyle(.borderless).menuIndicator(.hidden).fixedSize()
                    }
                    ForEach(t.notes, id: \.self) { n in Text("› \(n)").font(.caption).foregroundStyle(.secondary).padding(.leading, 30) }
                    if noting == "\(o.slug)#\(t.line)" {
                        TextField("Note — Return to add", text: $noteText)
                            .onSubmit { apply(o.slug, ["op": "note", "line": t.line, "text": t.text, "note": noteText]); noting = nil }
                            .padding(.leading, 30)
                    }
                }
            }
            TextField("Add a task — Return", text: Binding(get: { adding[o.slug] ?? "" }, set: { adding[o.slug] = $0 }))
                .onSubmit {
                    let t = (adding[o.slug] ?? "").trimmingCharacters(in: .whitespaces)
                    guard !t.isEmpty else { return }
                    adding[o.slug] = ""
                    apply(o.slug, ["op": "add", "text": t])
                }
            let on = model.sessions.filter { $0.meta.objective == o.slug && $0.state.state != "ended" }
            if !on.isEmpty {
                Text("Sessions on it").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                ForEach(on) { chip($0) }
            }
        }
        .padding(18)
        .background(.background.secondary, in: .rect(cornerRadius: 16))
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Board").font(.title.weight(.bold))
                    Text("Objectives your agents work on, and what each session is doing. Ticking or noting a task reaches the agents on it with their next prompt.")
                        .foregroundStyle(.secondary)
                }
                if !model.waiting.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Needs you (\(model.waiting.count))").font(.headline).foregroundStyle(.orange)
                        ForEach(model.waiting) { chip($0) }
                    }
                }
                if model.objectives.isEmpty {
                    Text("No objectives yet. An agent starts one with `claude-deck canvas attach <session dir> <slug> \"Title\"`.")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.objectives.filter { $0.status != "done" }) { card($0) }
                let others = model.sessions.filter { $0.meta.objective.isEmpty && !$0.state.state.isEmpty && $0.state.state != "ended" && !$0.needsYou }
                if !others.isEmpty {
                    Text("Other sessions").font(.headline)
                    ForEach(others) { chip($0) }
                }
                let done = model.objectives.filter { $0.status == "done" }
                if !done.isEmpty {
                    DisclosureGroup("Done (\(done.count))") { VStack(spacing: 12) { ForEach(done) { card($0) } }.padding(.top, 8) }
                }
            }
            .padding(.horizontal, 28).padding(.vertical, 24)
            .frame(maxWidth: 980, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .navigationTitle("Board")
    }
}

// MARK: - Jira

enum JiraPalette {
    static func category(_ c: String?) -> Color {
        switch c { case "done": return .green; case "indeterminate": return .blue; default: return .gray }
    }
}

@MainActor
final class JiraModel: ObservableObject {
    enum Mode: String, CaseIterable { case mine = "Mine", board = "Board", epics = "Epics", search = "Search" }

    @Published var status: JiraStatusInfo?
    @Published var mode: Mode = .mine
    @Published var issues: [JiraIssue] = []
    @Published var boards: [JiraBoardRef] = []
    @Published var boardId: Int? = UserDefaults.standard.object(forKey: "jiraBoard") as? Int {
        didSet { UserDefaults.standard.set(boardId, forKey: "jiraBoard") }
    }
    @Published var scope = "sprint"
    @Published var board: JiraBoard?
    @Published var epics: [JiraEpic] = []
    @Published var jql = ""
    @Published var filter = ""
    @Published var selected: String?
    @Published var detail: JiraDetail?
    @Published var loading = false
    @Published var error: String?

    func start() async {
        do { status = try await CanvasCLI.jira(JiraStatusInfo.self, ["status"]) } catch { self.error = error.localizedDescription }
        if status?.configured == true {
            boards = (try? await CanvasCLI.jira([JiraBoardRef].self, ["boards"])) ?? []
            await load()
        }
    }

    func load() async {
        guard status?.configured == true else { return }
        loading = true
        defer { loading = false }
        do {
            switch mode {
            case .mine: issues = try await CanvasCLI.jira([JiraIssue].self, ["mine"])
            case .search: issues = jql.isEmpty ? [] : try await CanvasCLI.jira([JiraIssue].self, ["search", jql])
            case .board:
                if let id = boardId ?? boards.first?.id {
                    boardId = id
                    board = try await CanvasCLI.jira(JiraBoard.self, ["board", String(id), "--scope", scope])
                }
            case .epics:
                let project = board?.project ?? boards.first(where: { $0.id == boardId })?.project ?? boards.first?.project ?? ""
                epics = project.isEmpty ? [] : try await CanvasCLI.jira([JiraEpic].self, ["epics", project])
            }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func open(_ key: String) async {
        selected = key
        detail = nil
        detail = try? await CanvasCLI.jira(JiraDetail.self, ["show", key])
    }

    /// A write; reloads the list and the open issue.
    func run(_ args: [String], flash: (String) -> Void) async {
        do { flash(try await CanvasCLI.jiraDo(args)) } catch { flash(error.localizedDescription) }
        await load()
        if let k = selected { await open(k) }
    }
}

struct JiraView: View {
    @StateObject var jira = JiraModel()
    @ObservedObject var canvas: CanvasModel
    @EnvironmentObject var state: AppState

    var body: some View {
        Group {
            if let s = jira.status, !s.configured {
                ContentUnavailableView {
                    Label("Jira isn't set up", systemImage: "key")
                } description: {
                    Text("Put JIRA_BASE_URL, JIRA_EMAIL and JIRA_API_TOKEN (KEY=value lines) in ~/.local/share/claude-deck/canvas/jira.env, then reload.")
                } actions: {
                    Button("Reload") { Task { await jira.start() } }
                }
            } else {
                HSplitView {
                    VStack(spacing: 0) {
                        toolbar
                        Divider()
                        if let e = jira.error { Text(e).foregroundStyle(.red).padding() }
                        content
                    }
                    .frame(minWidth: 480)
                    if jira.selected != nil {
                        JiraDetailView(jira: jira).frame(minWidth: 340, idealWidth: 420, maxWidth: 560)
                    }
                }
            }
        }
        .navigationTitle("Jira")
        .navigationSubtitle(jira.status?.user.map { "\($0.name) · \(jira.status?.base ?? "")" } ?? "")
        .task { await jira.start() }
    }

    private var toolbar: some View {
        HStack(spacing: 10) {
            Picker("", selection: $jira.mode) { ForEach(JiraModel.Mode.allCases, id: \.self) { Text($0.rawValue).tag($0) } }
                .pickerStyle(.segmented).labelsHidden().fixedSize()
                .onChange(of: jira.mode) { _, _ in Task { await jira.load() } }
            if jira.mode == .board || jira.mode == .epics {
                Picker("", selection: Binding(get: { jira.boardId ?? -1 }, set: { jira.boardId = $0; Task { await jira.load() } })) {
                    ForEach(jira.boards) { b in Text(b.name).tag(b.id) }
                }
                .labelsHidden().frame(maxWidth: 220)
            }
            if jira.mode == .board {
                Picker("", selection: $jira.scope) { Text("Sprint").tag("sprint"); Text("Backlog").tag("backlog"); Text("All").tag("all") }
                    .labelsHidden().fixedSize()
                    .onChange(of: jira.scope) { _, _ in Task { await jira.load() } }
            }
            if jira.mode == .search {
                TextField("JQL, e.g. project = RAID AND status = \"In Progress\"", text: $jira.jql)
                    .onSubmit { Task { await jira.load() } }
            } else {
                TextField("Filter", text: $jira.filter).frame(maxWidth: 200)
            }
            Spacer()
            if jira.loading { ProgressView().controlSize(.small) }
            Button { Task { await jira.load() } } label: { Image(systemName: "arrow.clockwise") }.buttonStyle(.borderless)
        }
        .padding(10)
    }

    private func matches(_ i: JiraIssue) -> Bool {
        let f = jira.filter.lowercased()
        return f.isEmpty || i.key.lowercased().contains(f) || i.summary.lowercased().contains(f) || i.assignee.lowercased().contains(f)
            || (i.epic?.summary.lowercased().contains(f) ?? false)
    }

    @ViewBuilder private var content: some View {
        switch jira.mode {
        case .mine, .search:
            List(jira.issues.filter(matches), selection: Binding(get: { jira.selected }, set: { k in if let k { Task { await jira.open(k) } } })) { i in
                IssueRow(issue: i).tag(i.key)
            }
        case .board:
            if let b = jira.board {
                VStack(alignment: .leading, spacing: 0) {
                    HStack {
                        Text(b.name).font(.headline)
                        if let sp = b.sprint { Text("· \(sp.name)").foregroundStyle(.secondary) }
                        Spacer()
                        Button("Dependency graph") { graph(keys: b.columns.flatMap(\.issues).filter(matches).map(\.key)) }
                    }
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    ScrollView(.horizontal) {
                        HStack(alignment: .top, spacing: 12) {
                            ForEach(b.columns) { col in column(col) }
                        }
                        .padding(12)
                    }
                }
            } else {
                ContentUnavailableView("Pick a board", systemImage: "rectangle.split.3x1")
            }
        case .epics:
            List(jira.epics) { e in
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text(e.key).font(.caption.monospaced()).foregroundStyle(.secondary)
                        Text(e.summary).fontWeight(.medium)
                        Spacer()
                        Text("\(e.done)/\(e.total) · \(Int(e.pointsDone))/\(Int(e.points)) pt").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                        if e.blocked > 0 { Text("\(e.blocked) blocked").font(.caption).foregroundStyle(.red) }
                        Menu {
                            Button("Open") { Task { await jira.open(e.key) } }
                            Button("Dependency graph on the canvas") { epicGraph(e.key) }
                            Button("Filter the board to it") { jira.filter = e.summary; jira.mode = .board; Task { await jira.load() } }
                        } label: { Image(systemName: "ellipsis") }
                        .menuStyle(.button).buttonStyle(.borderless).menuIndicator(.hidden).fixedSize()
                    }
                    ProgressView(value: e.total == 0 ? 0 : Double(e.done) / Double(e.total)).tint(.green)
                }
                .padding(.vertical, 4)
            }
        }
    }

    private func column(_ col: JiraColumn) -> some View {
        let issues = col.issues.filter(matches)
        let pts = issues.compactMap(\.points).reduce(0, +)
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(col.name).font(.subheadline.weight(.semibold))
                Text("\(issues.count)\(pts > 0 ? " · \(Int(pts)) pt" : "")").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            ScrollView {
                VStack(spacing: 8) {
                    ForEach(issues) { i in
                        IssueCard(issue: i, selected: jira.selected == i.key)
                            .onTapGesture { Task { await jira.open(i.key) } }
                            .draggable(i.key)
                    }
                }
            }
        }
        .padding(10)
        .frame(width: 270)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(.background.secondary, in: .rect(cornerRadius: 12))
        .dropDestination(for: String.self) { keys, _ in
            guard let key = keys.first, let target = col.statuses.first,
                  !(col.issues.contains { $0.key == key }) else { return false }
            Task { await jira.run(["move", key, target], flash: state.flash) }
            return true
        }
    }

    private var sessionDir: String? {
        canvas.sessions.first { $0.state.state != "ended" }?.dir ?? canvas.sessions.first?.dir
    }

    private func epicGraph(_ key: String) {
        guard let dir = sessionDir else { state.flash("Start an agent session first: the graph goes on its canvas"); return }
        Task {
            do { state.flash(try await CanvasCLI.jiraDo(["graph", key, dir])); state.selection = .canvas } catch { state.flash(error.localizedDescription) }
        }
    }

    private func graph(keys: [String]) {
        guard let dir = sessionDir else { state.flash("Start an agent session first: the graph goes on its canvas"); return }
        Task {
            do { state.flash(try await CanvasCLI.jiraDo(["graph-keys", keys.joined(separator: ","), dir, "board-deps"])); state.selection = .canvas }
            catch { state.flash(error.localizedDescription) }
        }
    }
}

struct IssueRow: View {
    let issue: JiraIssue
    var body: some View {
        HStack(spacing: 10) {
            Text(issue.type.prefix(1)).font(.caption.bold()).frame(width: 18, height: 18)
                .background(DeckTheme.accent.opacity(0.18), in: .rect(cornerRadius: 4))
            Text(issue.key).font(.callout.monospaced()).foregroundStyle(.secondary)
            Text(issue.summary).lineLimit(1)
            Spacer()
            Text(issue.status).font(.caption.weight(.semibold))
                .padding(.horizontal, 6).padding(.vertical, 1)
                .background(JiraPalette.category(issue.category).opacity(0.18), in: Capsule())
            Text(issue.assignee).font(.caption).foregroundStyle(.secondary).frame(width: 110, alignment: .trailing).lineLimit(1)
        }
    }
}

struct IssueCard: View {
    let issue: JiraIssue
    let selected: Bool

    private var initials: String {
        issue.assignee.split(separator: " ").prefix(2).compactMap(\.first).map(String.init).joined()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(issue.summary).font(.callout).lineLimit(3)
            HStack(spacing: 6) {
                Text(issue.type).font(.caption2).foregroundStyle(.secondary)
                Text(issue.key).font(.caption2.monospaced()).foregroundStyle(.secondary)
                if let e = issue.epic { Text(e.key).font(.caption2).foregroundStyle(DeckTheme.accent) }
                if issue.blocked == true { Image(systemName: "nosign").font(.caption2).foregroundStyle(.red).help("Blocked by \(issue.blockedBy?.joined(separator: ", ") ?? "")") }
                Spacer()
                if let d = issue.duedate, !d.isEmpty { Text(d).font(.caption2).foregroundStyle(.secondary) }
                if let p = issue.points { Text(String(format: "%g", p)).font(.caption2.monospacedDigit()).padding(.horizontal, 5).background(.quaternary, in: Capsule()) }
                if !initials.isEmpty {
                    Text(initials).font(.caption2.bold()).frame(width: 20, height: 20).background(.quaternary, in: Circle())
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background, in: .rect(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(selected ? DeckTheme.accent : Color.primary.opacity(0.08), lineWidth: selected ? 1.5 : 1))
    }
}

struct JiraDetailView: View {
    @ObservedObject var jira: JiraModel
    @EnvironmentObject var state: AppState
    @State var summary = ""
    @State var points = ""
    @State var labels = ""
    @State var due = ""
    @State var description = ""
    @State var comment = ""
    @State var linkKind = "blocks"
    @State var linkKey = ""

    private func run(_ args: [String]) { Task { await jira.run(args, flash: state.flash) } }

    var body: some View {
        ScrollView {
            if let d = jira.detail {
                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text(d.key).font(.callout.monospaced()).foregroundStyle(.secondary)
                        Text(d.type).font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        if let u = d.url, let url = URL(string: u) { Link(destination: url) { Image(systemName: "safari") } }
                        Button { jira.selected = nil; jira.detail = nil } label: { Image(systemName: "xmark") }.buttonStyle(.borderless)
                    }
                    TextField("Summary", text: $summary, axis: .vertical).font(.title3.weight(.semibold))
                        .onSubmit { if summary != d.summary { run(["edit", d.key, "--summary", summary]) } }
                    HStack {
                        Menu(d.status) {
                            ForEach(d.transitions ?? []) { t in Button(t.to) { run(["move", d.key, t.id]) } }
                        }
                        .fixedSize()
                        Button("Assign to me") { run(["assign", d.key, "me"]) }
                        Spacer()
                        Text(d.assignee.isEmpty ? "Unassigned" : d.assignee).foregroundStyle(.secondary)
                    }
                    Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                        if d.pointsField == true {
                            GridRow {
                                Text("Points").foregroundStyle(.secondary)
                                TextField("–", text: $points).frame(width: 80).onSubmit { run(["edit", d.key, "--points", points]) }
                            }
                        }
                        GridRow {
                            Text("Labels").foregroundStyle(.secondary)
                            TextField("a, b", text: $labels).onSubmit { run(["edit", d.key, "--labels", labels]) }
                        }
                        GridRow {
                            Text("Due").foregroundStyle(.secondary)
                            TextField("YYYY-MM-DD", text: $due).frame(width: 120).onSubmit { run(["edit", d.key, "--due", due]) }
                        }
                    }
                    Text("Description").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    TextEditor(text: $description).frame(minHeight: 100).scrollContentBackground(.hidden)
                        .padding(6).background(.background.secondary, in: .rect(cornerRadius: 8))
                    if description != (d.description ?? "") {
                        Button("Save description") { run(["edit", d.key, "--description", description]) }
                    }
                    Text("Links").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(d.links ?? []) { l in
                        HStack {
                            Text(l.label).foregroundStyle(.secondary)
                            Button(l.key) { Task { await jira.open(l.key) } }.buttonStyle(.link)
                            Text(l.summary).lineLimit(1)
                            Spacer()
                            Text(l.status).font(.caption).foregroundStyle(.secondary)
                            Button { run(["unlink", d.key, l.key]) } label: { Image(systemName: "xmark.circle") }.buttonStyle(.borderless)
                        }
                    }
                    HStack {
                        Picker("", selection: $linkKind) {
                            ForEach(["blocks", "blocked-by", "relates", "duplicates"], id: \.self) { Text($0).tag($0) }
                        }
                        .labelsHidden().fixedSize()
                        TextField("KEY-1", text: $linkKey).frame(width: 100)
                        Button("Link") { run(["link", d.key, linkKind, linkKey.uppercased()]); linkKey = "" }.disabled(linkKey.isEmpty)
                    }
                    if let subs = d.subtasks, !subs.isEmpty {
                        Text("Subtasks").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                        ForEach(subs) { s in IssueRow(issue: s).onTapGesture { Task { await jira.open(s.key) } } }
                    }
                    Text("Comments").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    ForEach(Array((d.comments ?? []).enumerated()), id: \.offset) { _, c in
                        VStack(alignment: .leading, spacing: 2) {
                            Text("\(c.author) · \(String(c.created.prefix(10)))").font(.caption).foregroundStyle(.secondary)
                            Text(c.text)
                        }
                    }
                    TextField("Add a comment — Return", text: $comment, axis: .vertical)
                        .onSubmit { let t = comment; comment = ""; if !t.isEmpty { run(["comment", d.key, t]) } }
                }
                .padding(16)
                .onAppear { sync(d) }
                .onChange(of: d.key) { _, _ in sync(d) }
            } else {
                ProgressView().padding(40)
            }
        }
    }

    private func sync(_ d: JiraDetail) {
        summary = d.summary
        points = d.points.map { String(format: "%g", $0) } ?? ""
        labels = (d.labels ?? []).joined(separator: ", ")
        due = d.duedate ?? ""
        description = d.description ?? ""
    }
}
