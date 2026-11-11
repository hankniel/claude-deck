import AppKit
import SwiftUI

/// Everything a screen needs from the canvas page.
struct ScreenContext {
    let session: CanvasSession
    let name: String
    let open: OpenHandler
    let toast: (String) -> Void
    var path: String { session.path(name) }
    var version: Int64 { session.items.first { $0.name == name }?.mtime ?? 0 }
}

func resolvePath(_ p: String, cwd: String) -> String {
    let p = p.trimmingCharacters(in: .whitespaces)
    if p.hasPrefix("~/") { return NSHomeDirectory() + String(p.dropFirst(1)) }
    if p.hasPrefix("/") || cwd.isEmpty { return p }
    return (cwd as NSString).appendingPathComponent(p)
}

/// Loads a parse result for a screen and shows a view of it.
struct Parsed<T: Decodable, V: View>: View {
    let file: String
    let version: Int64
    let content: (T) -> V
    @State private var value: T?
    @State private var error: String?

    init(file: String, version: Int64, @ViewBuilder content: @escaping (T) -> V) {
        self.file = file
        self.version = version
        self.content = content
    }

    var body: some View {
        Group {
            if let v = value { content(v) }
            else if let e = error { ContentUnavailableView("Can't show this screen", systemImage: "exclamationmark.triangle", description: Text(e)) }
            else { ProgressView() }
        }
        .task(id: version) {
            do { value = try await CanvasCLI.parse(T.self, file); error = nil } catch { self.error = error.localizedDescription }
        }
    }
}

private struct Page<C: View>: View {
    @ViewBuilder let content: () -> C
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) { content() }
                .padding(.horizontal, 28).padding(.vertical, 22)
                .frame(maxWidth: 920, alignment: .leading)
                .frame(maxWidth: .infinity)
        }
    }
}

// MARK: - .rv

struct ReviewScreen: View {
    let ctx: ScreenContext
    let doc: ReviewDoc
    @State var kept: Set<String> = []
    @State var note = ""

    private func color(_ s: String) -> Color {
        switch s { case "high": return .red; case "medium": return .orange; default: return .secondary }
    }

    var body: some View {
        Page {
            Text(doc.title.isEmpty ? "Review" : doc.title).font(.title2.weight(.semibold))
            Text("\(doc.findings.count) findings — keep the ones worth fixing, then send. The agent applies only those.")
                .foregroundStyle(.secondary)
            VStack(spacing: 0) {
                ForEach(doc.findings) { f in
                    HStack(alignment: .top, spacing: 12) {
                        Toggle("", isOn: Binding(get: { kept.contains(f.key) }, set: { on in if on { kept.insert(f.key) } else { kept.remove(f.key) } }))
                            .labelsHidden().toggleStyle(.checkbox)
                        Text(f.severity).font(.caption.weight(.bold)).foregroundStyle(color(f.severity))
                            .padding(.horizontal, 8).padding(.vertical, 2)
                            .background(color(f.severity).opacity(0.14), in: Capsule())
                            .frame(minWidth: 64)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(f.message).fontWeight(.medium)
                            if !f.fix.isEmpty { Text(f.fix).foregroundStyle(.secondary) }
                            if !f.file.isEmpty {
                                Button(f.line.map { "\(f.file):\($0)" } ?? f.file) { ctx.open(resolvePath(f.file, cwd: ctx.session.meta.cwd), f.line) }
                                    .buttonStyle(.link).font(.callout.monospaced())
                            }
                        }
                        Spacer()
                    }
                    .padding(12)
                    .contentShape(Rectangle())
                    .onTapGesture { if kept.contains(f.key) { kept.remove(f.key) } else { kept.insert(f.key) } }
                    Divider()
                }
            }
            .background(.background.secondary, in: .rect(cornerRadius: 12))
            Text("Anything to add (optional)").font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $note).frame(minHeight: 60).scrollContentBackground(.hidden)
                .padding(6).background(.background.secondary, in: .rect(cornerRadius: 8))
            HStack {
                Button("Send to agent") {
                    let keys = doc.findings.map(\.key).filter { kept.contains($0) }
                    Task {
                        do {
                            try await CanvasCLI.answer(dir: ctx.session.dir, screen: ctx.name, choices: keys, note: note.trimmingCharacters(in: .whitespacesAndNewlines))
                            ctx.toast("Sent \(keys.count) kept finding\(keys.count == 1 ? "" : "s") to the agent")
                        } catch { ctx.toast(error.localizedDescription) }
                    }
                }
                .buttonStyle(.glassProminent).tint(DeckTheme.accent)
                Button("Keep all") { kept = Set(doc.findings.map(\.key)) }
            }
        }
    }
}

// MARK: - .rows

struct RowsScreen: View {
    let doc: RowsDoc
    let name: String

    var body: some View {
        let maxV = max(doc.steps.flatMap { [$0.in ?? 0, $0.out ?? 0] }.max() ?? 1, 1)
        Page {
            Text(doc.title.isEmpty ? name : doc.title).font(.title2.weight(.semibold))
            ForEach(doc.steps) { s in
                let delta: Double? = (s.in != nil && s.out != nil) ? s.out! - s.in! : nil
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text(s.name).fontWeight(.semibold)
                        Spacer()
                        Text(s.out.map { "\(CanvasFormat.number($0)) rows" + (s.cols.map { " · \(Int($0)) cols" } ?? "") } ?? "—")
                            .monospacedDigit().foregroundStyle(.secondary)
                        if let d = delta, d != 0 {
                            let pct = (s.in ?? 0) != 0 ? d / s.in! * 100 : 0
                            Text("\(d > 0 ? "+" : "")\(CanvasFormat.number(d)) (\(String(format: "%+.1f", pct))%)")
                                .monospacedDigit().fontWeight(.semibold).foregroundStyle(d < 0 ? .red : .green)
                        } else {
                            Text("unchanged").foregroundStyle(.secondary)
                        }
                    }
                    GeometryReader { g in
                        let w = g.size.width
                        let kept = (s.out ?? 0) / maxV
                        let lost = delta.map { $0 < 0 ? -$0 / maxV : 0 } ?? 0
                        let gain = delta.map { $0 > 0 ? $0 / maxV : 0 } ?? 0
                        ZStack(alignment: .leading) {
                            Capsule().fill(.quaternary)
                            HStack(spacing: 0) {
                                Rectangle().fill(DeckTheme.accent.opacity(0.85)).frame(width: w * max(kept - gain, 0))
                                Rectangle().fill(Color.green.opacity(0.85)).frame(width: w * gain)
                                Rectangle().fill(Color.red.opacity(0.55)).frame(width: w * lost)
                            }
                            .clipShape(Capsule())
                        }
                    }
                    .frame(height: 10)
                }
            }
        }
    }
}

// MARK: - Markdown & .doc

struct MarkdownBlock: View {
    let text: String
    var body: some View {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if t.hasPrefix("```") {
            let code = t.split(separator: "\n", omittingEmptySubsequences: false).dropFirst().filter { !$0.hasPrefix("```") }.joined(separator: "\n")
            Text(code).font(.system(.callout, design: .monospaced)).textSelection(.enabled)
                .padding(10).frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 8))
        } else if let h = t.firstIndex(where: { $0 != "#" }), t.hasPrefix("#"), t[h] == " " {
            let level = t.distance(from: t.startIndex, to: h)
            Text(inline(String(t[h...]).trimmingCharacters(in: .whitespaces)))
                .font(level == 1 ? .title.bold() : level == 2 ? .title2.bold() : .title3.bold())
        } else {
            Text(inline(t))
        }
    }

    private func inline(_ s: String) -> AttributedString {
        (try? AttributedString(markdown: s, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(s)
    }
}

struct MarkdownScreen: View {
    let blocks: [String]
    var body: some View {
        Page { ForEach(Array(blocks.enumerated()), id: \.offset) { _, b in MarkdownBlock(text: b) } }
    }
}

struct DocScreen: View {
    let ctx: ScreenContext
    let path: String
    @State var blocks: [String] = []
    @State var error: String?
    @State var editing: Int?
    @State var draft = ""
    @State var comments: [(Int, String, String)] = []

    var body: some View {
        Page {
            Text((path as NSString).abbreviatingWithTildeInPath).font(.caption).foregroundStyle(.secondary)
            Text("Click a paragraph to comment on it; send the comments to the agent when done.").foregroundStyle(.secondary)
            if let e = error { Text(e).foregroundStyle(.red) }
            ForEach(Array(blocks.enumerated()), id: \.offset) { i, b in
                VStack(alignment: .leading, spacing: 6) {
                    MarkdownBlock(text: b)
                        .padding(.leading, comments.contains { $0.0 == i + 1 } ? 8 : 0)
                        .overlay(alignment: .leading) {
                            if comments.contains(where: { $0.0 == i + 1 }) { Rectangle().fill(DeckTheme.accent).frame(width: 3) }
                        }
                        .contentShape(Rectangle())
                        .onTapGesture { editing = i; draft = "" }
                    if editing == i {
                        TextField("Comment on this paragraph — Return to add", text: $draft)
                            .onSubmit {
                                let t = draft.trimmingCharacters(in: .whitespaces)
                                if !t.isEmpty {
                                    let ex = String(b.replacingOccurrences(of: "#", with: "").replacingOccurrences(of: "\n", with: " ").prefix(60)).trimmingCharacters(in: .whitespaces)
                                    comments.append((i + 1, ex, t))
                                }
                                editing = nil
                            }
                            .onExitCommand { editing = nil }
                    }
                }
            }
            if !comments.isEmpty {
                Divider()
                ForEach(Array(comments.enumerated()), id: \.offset) { _, c in
                    (Text("¶\(c.0) ").bold() + Text(c.2) + Text(" — \(c.1)…").foregroundStyle(.secondary))
                }
                Button("Send comments to agent") {
                    let choices = comments.map { "¶\($0.0)" }
                    let note = comments.map { "¶\($0.0) (\($0.1)…): \($0.2)" }.joined(separator: "\n")
                    Task {
                        do { try await CanvasCLI.answer(dir: ctx.session.dir, screen: ctx.name, choices: choices, note: note); ctx.toast("Comments sent to the agent") }
                        catch { ctx.toast(error.localizedDescription) }
                    }
                }
                .buttonStyle(.glassProminent).tint(DeckTheme.accent)
            }
        }
        .task(id: path) {
            guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { error = "Couldn't read \(path)"; return }
            blocks = Self.split(text)
        }
    }

    static func split(_ md: String) -> [String] {
        var out: [String] = [], cur: [String] = [], fence = false
        for line in md.components(separatedBy: "\n") {
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
                fence.toggle(); cur.append(line)
                if !fence { out.append(cur.joined(separator: "\n")); cur = [] }
                continue
            }
            if !fence && line.trimmingCharacters(in: .whitespaces).isEmpty {
                if !cur.isEmpty { out.append(cur.joined(separator: "\n")); cur = [] }
                continue
            }
            cur.append(line)
        }
        if !cur.isEmpty { out.append(cur.joined(separator: "\n")) }
        return out
    }
}

// MARK: - files and folders

struct CodeTextView: NSViewRepresentable {
    let text: String
    let line: Int?

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        let tv = scroll.documentView as! NSTextView
        tv.isEditable = false
        tv.isRichText = false
        tv.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        tv.textContainerInset = NSSize(width: 12, height: 10)
        tv.isHorizontallyResizable = true
        tv.textContainer?.widthTracksTextView = false
        tv.textContainer?.containerSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        scroll.hasHorizontalScroller = true
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let tv = scroll.documentView as? NSTextView, tv.string != text else { return }
        tv.string = text
        tv.textColor = .labelColor
        guard let line, line > 0 else { return }
        let ns = text as NSString
        var loc = 0
        for _ in 1..<line {
            let r = ns.range(of: "\n", range: NSRange(location: loc, length: ns.length - loc))
            if r.location == NSNotFound { break }
            loc = r.location + 1
        }
        let range = ns.lineRange(for: NSRange(location: min(loc, ns.length), length: 0))
        tv.textStorage?.addAttribute(.backgroundColor, value: NSColor.systemOrange.withAlphaComponent(0.22), range: range)
        DispatchQueue.main.async { tv.scrollRangeToVisible(range); tv.setSelectedRange(NSRange(location: range.location, length: 0)) }
    }
}

struct FileScreen: View {
    let path: String
    let line: Int?
    let open: OpenHandler

    var body: some View {
        var isDir: ObjCBool = false
        let exists = FileManager.default.fileExists(atPath: path, isDirectory: &isDir)
        if !exists {
            ContentUnavailableView("Not found", systemImage: "questionmark.folder", description: Text(path))
        } else if isDir.boolValue {
            let entries = ((try? FileManager.default.contentsOfDirectory(atPath: path)) ?? [])
                .filter { !$0.hasPrefix(".") }
                .map { name -> (String, Bool) in
                    var d: ObjCBool = false
                    FileManager.default.fileExists(atPath: (path as NSString).appendingPathComponent(name), isDirectory: &d)
                    return (name, d.boolValue)
                }
                .sorted { $0.1 != $1.1 ? $0.1 : $0.0.localizedStandardCompare($1.0) == .orderedAscending }
            List {
                Section((path as NSString).abbreviatingWithTildeInPath) {
                    Button { open((path as NSString).deletingLastPathComponent, nil) } label: { Label("..", systemImage: "arrow.up") }
                        .buttonStyle(.plain)
                    ForEach(entries, id: \.0) { e in
                        Button { open((path as NSString).appendingPathComponent(e.0), nil) } label: {
                            Label(e.0, systemImage: e.1 ? "folder" : "doc.text")
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        } else {
            let size = (try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int) ?? 0
            VStack(spacing: 0) {
                HStack {
                    Text((path as NSString).abbreviatingWithTildeInPath).font(.caption).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                    Spacer()
                    Button { NSWorkspace.shared.open(URL(fileURLWithPath: path)) } label: { Image(systemName: "arrow.up.forward.app") }
                        .buttonStyle(.borderless).help("Open with the default app")
                }
                .padding(.horizontal, 14).padding(.vertical, 6)
                Divider()
                if size > 4 * 1024 * 1024 {
                    ContentUnavailableView("Too large", systemImage: "doc", description: Text("\(path) is larger than 4 MB"))
                } else {
                    CodeTextView(text: (try? String(contentsOfFile: path, encoding: .utf8)) ?? "(binary file)", line: line)
                }
            }
        }
    }
}

struct SourceScreen: View {
    let path: String
    let note: String
    var browser = false
    var body: some View {
        Page {
            Text(note).foregroundStyle(.secondary)
            if browser {
                Button("Open in browser") { NSWorkspace.shared.open(URL(fileURLWithPath: path)) }.buttonStyle(.glass)
            }
            Text((try? String(contentsOfFile: path, encoding: .utf8)) ?? "").font(.system(.callout, design: .monospaced))
                .textSelection(.enabled).padding(10).frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 8))
        }
    }
}

// MARK: - .plan

struct PlanScreen: View {
    let ctx: ScreenContext
    let doc: PlanDoc
    @State var confirm = false
    @State var busy = false
    @State var commenting: Int?
    @State var draft = ""

    private func chip(_ key: String) -> some View {
        Group {
            if key.isEmpty {
                Text("new").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                    .padding(.horizontal, 6).padding(.vertical, 1).background(.quaternary, in: Capsule())
            } else {
                let live = doc.live[key]
                Button {
                    if let u = URL(string: "\(doc.base)/browse/\(key)") { NSWorkspace.shared.open(u) }
                } label: {
                    Text(key + (live?.status.map { " · \($0)" } ?? "")).font(.caption2.weight(.semibold))
                        .padding(.horizontal, 6).padding(.vertical, 1)
                        .background(JiraPalette.category(live?.category).opacity(0.18), in: Capsule())
                }
                .buttonStyle(.plain)
            }
        }
    }

    var body: some View {
        let p = doc.plan
        Page {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(p.epic.summary.isEmpty ? p.epic.key : p.epic.summary).font(.title2.weight(.semibold))
                    HStack(spacing: 8) {
                        Text("\(p.project) · Epic").foregroundStyle(.secondary)
                        chip(p.epic.key)
                        Text("\(Int(p.total)) pt").monospacedDigit().foregroundStyle(.secondary)
                    }
                }
                Spacer()
                Button("Dependency graph") {
                    Task {
                        do { ctx.toast(try await CanvasCLI.jiraDo(["plan-graph", ctx.path])) } catch { ctx.toast(error.localizedDescription) }
                    }
                }
                Button(busy ? "Pushing…" : "Push to Jira") { confirm = true }
                    .buttonStyle(.glassProminent).tint(DeckTheme.accent)
                    .disabled(busy || !p.errors.isEmpty)
            }
            ForEach(p.errors, id: \.self) { e in Label(e, systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
            if !p.epic.description.isEmpty { Text(p.epic.description.joined(separator: "\n")).foregroundStyle(.secondary) }
            ForEach(p.items) { it in
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 8) {
                        Text("#\(it.n)").monospacedDigit().foregroundStyle(.tertiary)
                        Text(it.type).font(.caption.weight(.bold)).foregroundStyle(DeckTheme.accent)
                        Text(it.summary).fontWeight(.semibold)
                        Spacer()
                        if let pts = it.points { Text("\(String(format: "%g", pts)) pt").monospacedDigit().foregroundStyle(.secondary) }
                        if !it.assignee.isEmpty { Text("@\(it.assignee)").foregroundStyle(.secondary) }
                        chip(it.key)
                        Button { commenting = it.n; draft = "" } label: { Image(systemName: "text.bubble") }
                            .buttonStyle(.borderless).help("Comment for the agent")
                    }
                    if !it.labels.isEmpty { Text(it.labels.joined(separator: ", ")).font(.caption).foregroundStyle(.secondary) }
                    if !it.description.isEmpty { Text(it.description.joined(separator: "\n")).font(.callout) }
                    if !it.ac.isEmpty {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Acceptance criteria").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                            ForEach(it.ac, id: \.self) { Text("• \($0)").font(.callout) }
                        }
                    }
                    ForEach(it.subtasks, id: \.line) { st in
                        HStack(spacing: 8) {
                            Text("↳ \(st.summary)").font(.callout)
                            Spacer()
                            if let pts = st.points { Text("\(String(format: "%g", pts)) pt").font(.caption).monospacedDigit().foregroundStyle(.secondary) }
                            chip(st.key)
                        }
                        .padding(.leading, 26)
                    }
                    if !it.after.isEmpty { Text("after \(it.after.joined(separator: ", "))").font(.caption).foregroundStyle(.secondary) }
                    if commenting == it.n {
                        TextField("Comment on #\(it.n) — Return to send to the agent", text: $draft)
                            .onSubmit {
                                let t = draft.trimmingCharacters(in: .whitespaces)
                                commenting = nil
                                guard !t.isEmpty else { return }
                                Task {
                                    do { try await CanvasCLI.answer(dir: ctx.session.dir, screen: ctx.name, choices: ["#\(it.n)"], note: "#\(it.n) \(it.summary): \(t)"); ctx.toast("Comment sent to the agent") }
                                    catch { ctx.toast(error.localizedDescription) }
                                }
                            }
                    }
                }
                .padding(14)
                .background(.background.secondary, in: .rect(cornerRadius: 12))
            }
        }
        .confirmationDialog("Create the new issues in \(p.project)?", isPresented: $confirm) {
            Button("Push to Jira") {
                busy = true
                Task {
                    do { ctx.toast(try await CanvasCLI.jiraDo(["push", ctx.path])) } catch { ctx.toast(error.localizedDescription) }
                    busy = false
                }
            }
        } message: {
            Text("Items without a key are created (epic, stories, subtasks) and `after:` becomes Blocks links. Existing issues are not edited.")
        }
    }
}

// MARK: - Git changes

struct GitChangesView: View {
    let session: CanvasSession
    @State var status: GitStatus?
    @State var selected: GitFile?
    @State var diff = ""
    @State var token = 0

    var body: some View {
        HSplitView {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    VStack(alignment: .leading) {
                        Text(status?.branch ?? "…").font(.headline)
                        if let s = status, s.error == nil {
                            Text("\(s.files?.count ?? 0) files · +\(s.added ?? 0) −\(s.deleted ?? 0)").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                    Spacer()
                    Button { token += 1 } label: { Image(systemName: "arrow.clockwise") }.buttonStyle(.borderless)
                }
                .padding(10)
                if let e = status?.error {
                    Text(e).foregroundStyle(.secondary).padding()
                }
                List(status?.files ?? [], selection: Binding(get: { selected?.path }, set: { p in selected = status?.files?.first { $0.path == p } })) { f in
                    HStack(spacing: 8) {
                        Text(f.letter).font(.caption.monospaced().bold())
                            .foregroundStyle(f.letter == "D" ? .red : f.letter == "U" || f.letter == "A" ? .green : .orange)
                        Text(f.path).lineLimit(1).truncationMode(.head)
                        Spacer()
                        Text("+\(f.added)").font(.caption.monospacedDigit()).foregroundStyle(.green)
                        Text("−\(f.deleted)").font(.caption.monospacedDigit()).foregroundStyle(.red)
                    }
                    .tag(f.path)
                }
            }
            .frame(minWidth: 240, idealWidth: 320)
            ScrollView([.vertical, .horizontal]) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(diff.split(separator: "\n", omittingEmptySubsequences: false).enumerated()), id: \.offset) { _, l in
                        let s = String(l)
                        Text(s.isEmpty ? " " : s).font(.system(size: 12, design: .monospaced))
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 10)
                            .background(s.hasPrefix("+") && !s.hasPrefix("+++") ? Color.green.opacity(0.14)
                                        : s.hasPrefix("-") && !s.hasPrefix("---") ? Color.red.opacity(0.14)
                                        : s.hasPrefix("@@") ? Color.blue.opacity(0.10) : .clear)
                    }
                }
                .textSelection(.enabled)
                .padding(.vertical, 8)
            }
            .overlay { if selected == nil { Text("Select a file").foregroundStyle(.secondary) } }
        }
        .task(id: token) { status = try? await CanvasCLI.git(session.meta.cwd) }
        .task(id: selected?.path) {
            guard let f = selected else { diff = ""; return }
            diff = (try? await CanvasCLI.diff(session.meta.cwd, file: f.path, untracked: f.untracked)) ?? ""
        }
    }
}
