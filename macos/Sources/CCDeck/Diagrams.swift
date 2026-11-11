import AppKit
import SwiftUI

/// Opens a file (path, line) or URL from a screen.
typealias OpenHandler = (String, Int?) -> Void

private struct DiagramError: View {
    let text: String
    var body: some View {
        ContentUnavailableView("Can't draw this screen", systemImage: "exclamationmark.triangle", description: Text(text))
    }
}

private func toolbarCapsule<C: View>(@ViewBuilder _ c: () -> C) -> some View {
    HStack(spacing: 8) { c() }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .glassEffect(.regular, in: .capsule)
}

// MARK: - Flowchart

struct FlowDiagramView: View {
    let file: String
    let version: Int64
    let open: OpenHandler

    @State var scene: FlowScene?
    @State var error: String?
    @State var collapsed: [String]?
    @State var hover: Int?
    @State var selected: Int?
    @State var search = ""
    @State var fitToken = 0

    private var key: String { "\(version)|\(collapsed?.joined(separator: ",") ?? "-")" }

    var body: some View {
        Group {
            if let s = scene {
                ZoomCanvas(contentSize: CGSize(width: s.width, height: s.height), minFit: 0.55, fitToken: fitToken,
                           events: DiagramEvents(hover: { hover = hit($0)?.0 }, tap: tap)) { ctx, t in
                    draw(&ctx, s, t)
                }
                .overlay(alignment: .topLeading) {
                    toolbarCapsule {
                        Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                        TextField("Find a node", text: $search).textFieldStyle(.plain).frame(width: 150)
                            .onSubmit { reveal() }
                        if s.nodes.contains(where: { $0.group }) {
                            Divider().frame(height: 16)
                            Button { collapsed = []; fitToken += 1 } label: { Image(systemName: "rectangle.expand.vertical") }
                                .help("Expand all groups")
                            Button { collapsed = ["*"]; fitToken += 1 } label: { Image(systemName: "rectangle.compress.vertical") }
                                .help("Collapse all groups")
                        }
                    }
                    .buttonStyle(.borderless)
                    .padding(12)
                }
                .overlay(alignment: .bottomLeading) {
                    Text("Click a node to trace it · click a group title to fold · drag to pan · ⌘-scroll to zoom")
                        .font(.caption).foregroundStyle(.secondary).padding(14)
                }
            } else if let e = error {
                DiagramError(text: e)
            } else {
                ProgressView()
            }
        }
        .task(id: key) { await load() }
    }

    private func load() async {
        do {
            let data = try await CanvasCLI.layout(file, collapsed: collapsed)
            scene = try JSONDecoder().decode(FlowScene.self, from: data)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func rect(_ i: Int?) -> CGRect? {
        guard let i, let s = scene, s.nodes.indices.contains(i) else { return nil }
        return s.nodes[i].rect?.cgRect
    }

    private func depth(_ i: Int) -> Int {
        var d = 0, p = scene?.nodes[i].parent
        while let x = p, d < 64 { d += 1; p = scene?.nodes[x].parent }
        return d
    }

    /// (node, isGroupTitle)
    private func hit(_ p: CGPoint?) -> (Int, Bool)? {
        guard let p, let s = scene else { return nil }
        var best: (Int, CGFloat)?
        for (i, n) in s.nodes.enumerated() where !(n.group && !n.collapsed) {
            if let r = n.rect?.cgRect, r.contains(p), best == nil || r.width * r.height < best!.1 { best = (i, r.width * r.height) }
        }
        if let b = best { return (b.0, false) }
        var title: (Int, CGFloat)?
        for (i, n) in s.nodes.enumerated() where n.group && !n.collapsed {
            if let r = n.rect?.cgRect, CGRect(x: r.minX, y: r.minY, width: r.width, height: 28).contains(p),
               title == nil || r.width * r.height < title!.1 { title = (i, r.width * r.height) }
        }
        return title.map { ($0.0, true) }
    }

    private var currentCollapsed: [String] {
        scene?.nodes.filter { $0.group && $0.collapsed }.map(\.id) ?? []
    }

    private func tap(_ p: CGPoint, _ clicks: Int, _ mods: NSEvent.ModifierFlags) {
        guard let s = scene else { return }
        guard let (i, isTitle) = hit(p) else { selected = nil; return }
        let n = s.nodes[i]
        if n.group {
            var c = currentCollapsed
            if isTitle { c.append(n.id) } else { c.removeAll { $0 == n.id } }
            collapsed = c
            return
        }
        if let link = n.link, clicks >= 2 || mods.contains(.command) {
            if !link.hasPrefix("http"), let colon = link.lastIndex(of: ":"), let line = Int(link[link.index(after: colon)...]) {
                open(String(link[..<colon]), line)
            } else {
                open(link, nil)
            }
            return
        }
        selected = selected == i ? nil : i
    }

    private func reveal() {
        guard let s = scene else { return }
        let q = search.lowercased().trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty else { return }
        var c = Set(currentCollapsed)
        for n in s.nodes where n.label.lowercased().contains(q) || n.id.lowercased().contains(q) {
            var p = n.parent
            while let x = p { c.remove(s.nodes[x].id); p = s.nodes[x].parent }
        }
        c.remove("*")
        collapsed = Array(c)
    }

    private func lit(_ s: FlowScene) -> (Set<Int>, Set<Int>) {
        guard let sel = selected else { return ([], []) }
        var nodes: Set<Int> = [sel], edges: Set<Int> = []
        for forward in [true, false] {
            var stack = [sel], seen: Set<Int> = [sel]
            while let x = stack.popLast() {
                for (k, e) in s.edges.enumerated() {
                    guard let a = e.from, let b = e.to, a != b else { continue }
                    let (from, to) = forward ? (a, b) : (b, a)
                    if from == x {
                        edges.insert(k)
                        if seen.insert(to).inserted { nodes.insert(to); stack.append(to) }
                    }
                }
            }
        }
        return (nodes, edges)
    }

    private func shapePath(_ shape: String, _ r: CGRect) -> Path {
        var p = Path()
        switch shape {
        case "round": p.addRoundedRect(in: r, cornerSize: CGSize(width: 12, height: 12))
        case "stadium": p.addRoundedRect(in: r, cornerSize: CGSize(width: r.height / 2, height: r.height / 2))
        case "circle": p.addEllipse(in: CGRect(x: r.midX - min(r.width, r.height) / 2, y: r.midY - min(r.width, r.height) / 2, width: min(r.width, r.height), height: min(r.width, r.height)))
        case "diamond":
            p.move(to: CGPoint(x: r.midX, y: r.minY)); p.addLine(to: CGPoint(x: r.maxX, y: r.midY))
            p.addLine(to: CGPoint(x: r.midX, y: r.maxY)); p.addLine(to: CGPoint(x: r.minX, y: r.midY)); p.closeSubpath()
        case "hexagon":
            let k: CGFloat = 14
            p.move(to: CGPoint(x: r.minX + k, y: r.minY)); p.addLine(to: CGPoint(x: r.maxX - k, y: r.minY))
            p.addLine(to: CGPoint(x: r.maxX, y: r.midY)); p.addLine(to: CGPoint(x: r.maxX - k, y: r.maxY))
            p.addLine(to: CGPoint(x: r.minX + k, y: r.maxY)); p.addLine(to: CGPoint(x: r.minX, y: r.midY)); p.closeSubpath()
        case "parallelogram":
            p.move(to: CGPoint(x: r.minX + 12, y: r.minY)); p.addLine(to: CGPoint(x: r.maxX, y: r.minY))
            p.addLine(to: CGPoint(x: r.maxX - 12, y: r.maxY)); p.addLine(to: CGPoint(x: r.minX, y: r.maxY)); p.closeSubpath()
        case "flag":
            p.move(to: CGPoint(x: r.minX, y: r.minY)); p.addLine(to: CGPoint(x: r.maxX, y: r.minY))
            p.addLine(to: CGPoint(x: r.maxX, y: r.maxY)); p.addLine(to: CGPoint(x: r.minX, y: r.maxY))
            p.addLine(to: CGPoint(x: r.minX + 12, y: r.midY)); p.closeSubpath()
        case "cylinder":
            let e: CGFloat = 7
            p.move(to: CGPoint(x: r.minX, y: r.minY + e))
            p.addCurve(to: CGPoint(x: r.maxX, y: r.minY + e), control1: CGPoint(x: r.minX, y: r.minY - e / 3), control2: CGPoint(x: r.maxX, y: r.minY - e / 3))
            p.addLine(to: CGPoint(x: r.maxX, y: r.maxY - e))
            p.addCurve(to: CGPoint(x: r.minX, y: r.maxY - e), control1: CGPoint(x: r.maxX, y: r.maxY + e / 3), control2: CGPoint(x: r.minX, y: r.maxY + e / 3))
            p.closeSubpath()
        default: p.addRoundedRect(in: r, cornerSize: CGSize(width: 6, height: 6))
        }
        return p
    }

    private func draw(_ ctx: inout GraphicsContext, _ s: FlowScene, _ t: DiagramTheme) {
        let (litNodes, litEdges) = lit(s)
        let dimming = selected != nil
        let q = search.lowercased().trimmingCharacters(in: .whitespaces)

        let groups = s.nodes.indices.filter { s.nodes[$0].group && !s.nodes[$0].collapsed && s.nodes[$0].rect != nil }
            .sorted { depth($0) < depth($1) }
        for g in groups {
            guard let r = rect(g) else { continue }
            let n = s.nodes[g]
            let base = Color(css: n.fill) ?? t.group
            let path = Path(roundedRect: r, cornerRadius: 10)
            ctx.fill(path, with: .color(n.fill != nil ? base.opacity(0.35) : base))
            ctx.stroke(path, with: .color(Color(css: n.stroke) ?? t.border), lineWidth: 1)
            Draw.text(&ctx, "▾  \(n.label)", in: CGRect(x: r.minX + 12, y: r.minY + 4, width: r.width - 24, height: 20),
                      size: 11, weight: .semibold, color: hover == g ? t.accent : t.muted)
        }

        for (k, e) in s.edges.enumerated() where e.kind != "invisible" {
            guard let ra = rect(e.from), let rb = rect(e.to) else { continue }
            let mids = (e.points ?? []).map { CGPoint(x: $0[0], y: $0[1]) }
            var pts = [Draw.boundary(ra, toward: mids.first ?? CGPoint(x: rb.midX, y: rb.midY))]
            pts += mids
            pts.append(Draw.boundary(rb, toward: mids.last ?? CGPoint(x: ra.midX, y: ra.midY)))
            let isLit = litEdges.contains(k)
            let hot = hover != nil && (hover == e.from || hover == e.to)
            let color = (isLit || hot ? t.accent : t.edge).opacity(dimming && !isLit ? 0.28 : 1)
            let style = StrokeStyle(lineWidth: e.kind == "thick" ? 2.6 : (isLit || hot ? 1.8 : 1.3), lineCap: .round,
                                    dash: e.kind == "dotted" ? [4, 4] : [])
            ctx.stroke(Draw.smooth(pts), with: .color(color), style: style)
            if e.arrowEnd { Draw.arrowHead(&ctx, tip: pts[pts.count - 1], from: pts[pts.count - 2], color: color) }
            if e.arrowStart { Draw.arrowHead(&ctx, tip: pts[0], from: pts[1], color: color) }
            if !e.label.isEmpty {
                let m = pts.count % 2 == 1 ? pts[pts.count / 2]
                    : CGPoint(x: (pts[pts.count / 2 - 1].x + pts[pts.count / 2].x) / 2, y: (pts[pts.count / 2 - 1].y + pts[pts.count / 2].y) / 2)
                Draw.pill(&ctx, e.label, center: m, fg: t.muted, bg: t.bg)
            }
        }

        for (i, n) in s.nodes.enumerated() {
            guard let r = n.rect?.cgRect, !(n.group && !n.collapsed) else { continue }
            let alpha = dimming && !litNodes.contains(i) ? 0.28 : 1.0
            var c = ctx
            c.opacity = alpha
            if n.group {
                let path = Path(roundedRect: r, cornerRadius: 10)
                c.fill(path, with: .color(t.group))
                c.stroke(path, with: .color(t.muted.opacity(0.8)), style: StrokeStyle(lineWidth: 1.2, dash: [5, 3]))
                Draw.text(&c, "▸  \(n.label)", in: CGRect(x: r.minX + 12, y: r.minY, width: r.width - 50, height: r.height), color: t.fg)
                let hidden = s.nodes.indices.filter { j in
                    var p = s.nodes[j].parent
                    while let x = p { if x == i { return !s.nodes[j].group }; p = s.nodes[x].parent }
                    return false
                }.count
                Draw.pill(&c, "+\(hidden)", center: CGPoint(x: r.maxX - 22, y: r.midY), fg: t.muted, bg: t.card)
            } else {
                let fill = Color(css: n.fill) ?? t.card
                let path = shapePath(n.shape, r)
                c.fill(path, with: .color(fill))
                let hot = hover == i || selected == i
                c.stroke(path, with: .color(hot ? t.accent : (Color(css: n.stroke) ?? t.border)),
                         style: StrokeStyle(lineWidth: hot ? 1.8 : 1, dash: n.dashed == true ? [4, 3] : []))
                let ink = Color(css: n.color) ?? (n.fill == nil ? t.fg : (fill.isLight ? Color(hex: 0x1F1E1D) : .white))
                c.draw(Text(n.label).font(.system(size: 13)).foregroundStyle(ink),
                       at: CGPoint(x: r.midX, y: n.shape == "cylinder" ? r.midY + 4 : r.midY), anchor: .center)
                if n.link != nil {
                    Draw.text(&c, "↗", at: CGPoint(x: r.maxX - 4, y: r.minY + 1), anchor: .topTrailing, size: 9, color: t.accent)
                }
            }
            if !q.isEmpty, n.label.lowercased().contains(q) || n.id.lowercased().contains(q) {
                c.stroke(Path(roundedRect: r.insetBy(dx: -4, dy: -4), cornerRadius: 9), with: .color(t.accent), lineWidth: 2.5)
            }
        }
    }
}

// MARK: - Lineage (Databricks Unity Catalog style)

enum LineageKind {
    static func color(_ k: String) -> Color {
        switch k {
        case "table": return Color(hex: 0x2272B4)
        case "view": return Color(hex: 0x7B61FF)
        case "mv", "pipeline": return Color(hex: 0x0E8A6D)
        case "streaming": return Color(hex: 0xE8730C)
        case "notebook": return Color(hex: 0xE0731A)
        case "job": return Color(hex: 0xD6455D)
        case "dashboard": return Color(hex: 0xA855C8)
        case "query": return Color(hex: 0x1F8FD1)
        case "model": return Color(hex: 0xC69100)
        default: return Color(hex: 0x6B7785)
        }
    }

    static func symbol(_ k: String) -> String {
        switch k {
        case "table": return "tablecells"
        case "view": return "eye"
        case "mv": return "tablecells.badge.ellipsis"
        case "streaming": return "dot.radiowaves.right"
        case "notebook": return "book.closed"
        case "job": return "point.3.connected.trianglepath.dotted"
        case "pipeline": return "arrow.triangle.branch"
        case "dashboard": return "chart.bar.xaxis"
        case "query": return "magnifyingglass"
        case "model": return "cube"
        case "volume", "file": return "doc"
        default: return "cylinder.split.1x2"
        }
    }

    static func label(_ k: String) -> String {
        ["table": "Table", "view": "View", "mv": "Materialized view", "streaming": "Streaming table", "notebook": "Notebook",
         "job": "Job", "pipeline": "Pipeline", "dashboard": "Dashboard", "query": "Query", "model": "Model",
         "volume": "Volume", "file": "File"][k] ?? "External"
    }
}

struct LineageDiagramView: View {
    let file: String
    let version: Int64

    @State var scene: LineageScene?
    @State var error: String?
    @State var expanded: [Int]?
    @State var hover: Int?
    @State var hoverCol: [Int]?
    @State var selected: Int?
    @State var selCol: [Int]?
    @State var fitToken = 0

    private var key: String { "\(version)|\(expanded?.map(String.init).joined(separator: ",") ?? "-")" }

    var body: some View {
        Group {
            if let s = scene {
                let anchor = s.focus.flatMap { s.nodes[$0].rect?.cgRect }.map { CGPoint(x: $0.midX, y: $0.midY) }
                ZoomCanvas(contentSize: CGSize(width: s.width, height: s.height), minFit: 0.62, anchor: anchor, fitToken: fitToken,
                           events: DiagramEvents(hover: hoverAt, tap: { p, _, _ in tap(p) })) { ctx, t in
                    draw(&ctx, s, t)
                }
                .overlay(alignment: .topLeading) {
                    toolbarCapsule {
                        if !s.title.isEmpty { Text(s.title).font(.headline) }
                        if s.nodes.contains(where: { !$0.columns.isEmpty }) {
                            Button { expanded = s.nodes.indices.filter { !s.nodes[$0].columns.isEmpty } } label: { Image(systemName: "list.bullet.rectangle") }
                                .help("Show all columns")
                            Button { expanded = []; selCol = nil } label: { Image(systemName: "rectangle.compress.vertical") }
                                .help("Hide columns")
                        }
                    }
                    .buttonStyle(.borderless)
                    .padding(12)
                }
                .overlay(alignment: .bottomLeading) {
                    Text("Point at an entity to see its upstream and downstream · click a column to trace it")
                        .font(.caption).foregroundStyle(.secondary).padding(14)
                }
            } else if let e = error {
                DiagramError(text: e)
            } else {
                ProgressView()
            }
        }
        .task(id: key) { await load() }
    }

    private func load() async {
        do {
            let data = try await CanvasCLI.layout(file, expanded: expanded)
            let s = try JSONDecoder().decode(LineageScene.self, from: data)
            scene = s
            if expanded == nil { expanded = s.nodes.indices.filter { s.nodes[$0].expanded } }
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func rect(_ i: Int) -> CGRect { scene?.nodes[i].rect?.cgRect ?? .zero }

    private enum Hit { case header(Int), toggle(Int), column(Int, Int) }

    private func hit(_ p: CGPoint) -> Hit? {
        guard let s = scene else { return nil }
        for (i, n) in s.nodes.enumerated() {
            let r = rect(i)
            guard r.contains(p) else { continue }
            if p.y < r.minY + s.head {
                return !n.columns.isEmpty && p.x > r.maxX - 40 ? .toggle(i) : .header(i)
            }
            if n.expanded {
                let c = Int((p.y - r.minY - s.head) / s.row)
                if n.columns.indices.contains(c) { return .column(i, c) }
            }
            return .header(i)
        }
        return nil
    }

    private func hoverAt(_ p: CGPoint?) {
        switch p.flatMap(hit) {
        case .header(let i), .toggle(let i): hover = i; hoverCol = nil
        case .column(let i, let c): hover = i; hoverCol = [i, c]
        case nil: hover = nil; hoverCol = nil
        }
    }

    private func related(_ s: LineageScene, _ i: Int) -> Set<Int> {
        var out: Set<Int> = [i]
        for forward in [true, false] {
            var stack = [i]
            while let x = stack.popLast() {
                for e in s.edges where e.count == 2 {
                    let (a, b) = forward ? (e[0], e[1]) : (e[1], e[0])
                    if a == x, out.insert(b).inserted { stack.append(b) }
                }
            }
        }
        return out
    }

    private func columnLit(_ s: LineageScene) -> Set<[Int]> {
        guard let at = selCol else { return [] }
        var seen: Set<[Int]> = [at]
        for forward in [true, false] {
            var stack = [at]
            while let x = stack.popLast() {
                for e in s.columnEdges where e.count == 4 {
                    let (from, to) = forward ? ([e[0], e[1]], [e[2], e[3]]) : ([e[2], e[3]], [e[0], e[1]])
                    if from == x, seen.insert(to).inserted { stack.append(to) }
                }
            }
        }
        return seen
    }

    private func tap(_ p: CGPoint) {
        guard let s = scene else { return }
        switch hit(p) {
        case .toggle(let i):
            var e = Set(expanded ?? [])
            if e.contains(i) { e.remove(i); if selCol?.first == i { selCol = nil } } else { e.insert(i) }
            expanded = Array(e).sorted()
        case .header(let i):
            selected = selected == i ? nil : i
        case .column(let i, let c):
            selCol = selCol == [i, c] ? nil : [i, c]
            let need = Set(columnLit(s).map { $0[0] })
            let e = Set(expanded ?? []).union(need)
            if e != Set(expanded ?? []) { expanded = Array(e).sorted() }
        case nil:
            selected = nil; selCol = nil
        }
    }

    private func draw(_ ctx: inout GraphicsContext, _ s: LineageScene, _ t: DiagramTheme) {
        let litSet = (selected ?? hover).map { related(s, $0) }
        let colLit = columnLit(s)
        for e in s.edges where e.count == 2 {
            let ra = rect(e[0]), rb = rect(e[1])
            let a = CGPoint(x: ra.maxX, y: ra.minY + s.head / 2), b = CGPoint(x: rb.minX - 1, y: rb.minY + s.head / 2)
            let hot = litSet.map { $0.contains(e[0]) && $0.contains(e[1]) } ?? false
            let color = (hot ? t.blue : t.edge).opacity(litSet != nil && !hot ? 0.18 : 1)
            ctx.stroke(Draw.sCurve(a, b), with: .color(color), lineWidth: hot ? 2 : 1.4)
            Draw.arrowHead(&ctx, tip: b, from: CGPoint(x: b.x - 10, y: b.y), color: color)
        }
        if selCol != nil {
            for e in s.columnEdges where e.count == 4 && colLit.contains([e[0], e[1]]) && colLit.contains([e[2], e[3]]) {
                guard s.nodes[e[0]].expanded, s.nodes[e[2]].expanded else { continue }
                let ra = rect(e[0]), rb = rect(e[2])
                let a = CGPoint(x: ra.maxX, y: ra.minY + s.head + CGFloat(e[1]) * s.row + s.row / 2)
                let b = CGPoint(x: rb.minX - 1, y: rb.minY + s.head + CGFloat(e[3]) * s.row + s.row / 2)
                ctx.stroke(Draw.sCurve(a, b), with: .color(t.blue), lineWidth: 1.8)
                Draw.arrowHead(&ctx, tip: b, from: CGPoint(x: b.x - 10, y: b.y), size: 7, color: t.blue)
            }
        }
        for (i, n) in s.nodes.enumerated() {
            let r = rect(i)
            var c = ctx
            c.opacity = litSet.map { $0.contains(i) ? 1 : 0.35 } ?? 1
            let focus = s.focus == i
            let card = Path(roundedRect: r, cornerRadius: 10)
            c.fill(Path(roundedRect: r.offsetBy(dx: 0, dy: 2), cornerRadius: 10), with: .color(.black.opacity(t.dark ? 0.35 : 0.07)))
            c.fill(card, with: .color(t.card))
            if focus {
                let head = Path(roundedRect: CGRect(x: r.minX, y: r.minY, width: r.width, height: n.expanded ? s.head : r.height), cornerRadius: 10)
                c.fill(head, with: .color(t.blueTint))
            }
            let hot = hover == i || selected == i
            c.stroke(Path(roundedRect: r.insetBy(dx: 0.5, dy: 0.5), cornerRadius: 10),
                     with: .color(focus || hot ? t.blue : t.border), lineWidth: focus ? 2 : (hot ? 1.6 : 1))
            let kc = LineageKind.color(n.kind)
            let tile = CGRect(x: r.minX + 13, y: r.minY + 13, width: 32, height: 32)
            c.fill(Path(roundedRect: tile, cornerRadius: 8), with: .color(kc.opacity(t.dark ? 0.22 : 0.11)))
            c.draw(Image(systemName: LineageKind.symbol(n.kind)), in: tile.insetBy(dx: 8, dy: 8))
            let textW = r.width - 56 - (n.columns.isEmpty ? 12 : 40)
            Draw.text(&c, n.namespace.isEmpty ? LineageKind.label(n.kind) : n.namespace,
                      in: CGRect(x: r.minX + 56, y: r.minY + 10, width: textW, height: 16), size: 11, color: t.muted)
            Draw.text(&c, n.short, in: CGRect(x: r.minX + 56, y: r.minY + 27, width: textW, height: 18), weight: .semibold, color: t.fg)
            if !n.columns.isEmpty {
                Draw.text(&c, "\(n.columns.count) \(n.expanded ? "▾" : "▸")", at: CGPoint(x: r.maxX - 12, y: r.minY + s.head / 2),
                          anchor: .trailing, size: 11, color: t.muted)
            }
            if n.expanded {
                var line = Path()
                line.move(to: CGPoint(x: r.minX, y: r.minY + s.head + 0.5)); line.addLine(to: CGPoint(x: r.maxX, y: r.minY + s.head + 0.5))
                c.stroke(line, with: .color(t.border), lineWidth: 1)
                for (ci, col) in n.columns.enumerated() {
                    let y = r.minY + s.head + CGFloat(ci) * s.row
                    let isSel = selCol == [i, ci], isLit = colLit.contains([i, ci]), isHov = hoverCol == [i, ci]
                    if isSel || isLit || isHov {
                        c.fill(Path(CGRect(x: r.minX + 1, y: y + 1, width: r.width - 2, height: s.row - 1)),
                               with: .color(isSel || isLit ? t.blueTint : t.cardAlt))
                    }
                    Draw.text(&c, col.name, in: CGRect(x: r.minX + 16, y: y, width: r.width * 0.6, height: s.row),
                              size: 11.5, color: isSel || isLit ? t.blue : t.fg)
                    if !col.type.isEmpty {
                        Draw.text(&c, col.type, in: CGRect(x: r.maxX - 14 - r.width * 0.35, y: y, width: r.width * 0.35, height: s.row),
                                  alignment: .trailing, size: 11, color: t.muted)
                    }
                }
            }
        }
    }
}

// MARK: - ERD (dbdiagram style)

struct ErdDiagramView: View {
    let file: String
    let version: Int64

    @State var scene: ErdScene?
    @State var error: String?
    @State var tables: [CGRect] = []
    @State var enums: [CGRect] = []
    @State var hoverTable: Int?
    @State var hoverRow: [Int]?
    @State var hoverRef: Int?
    @State var dragging: [(Bool, Int)] = []
    @State var fitToken = 0

    private var posFile: String {
        let u = URL(fileURLWithPath: file)
        return u.deletingLastPathComponent().appendingPathComponent(".\(u.lastPathComponent).pos").path
    }

    var body: some View {
        Group {
            if let s = scene {
                let b = bounds()
                ZoomCanvas(contentSize: CGSize(width: b.width, height: b.height), minFit: 0.6, fitToken: fitToken,
                           events: DiagramEvents(hover: { hoverAt($0.map { CGPoint(x: $0.x + b.minX, y: $0.y + b.minY) }) },
                                                 grab: { grab(CGPoint(x: $0.x + b.minX, y: $0.y + b.minY)) },
                                                 drag: move, drop: save)) { ctx, t in
                    ctx.translateBy(x: -b.minX, y: -b.minY)
                    draw(&ctx, s, t)
                }
                .overlay(alignment: .topLeading) {
                    toolbarCapsule {
                        Text("\(s.project.map { "\($0) · " } ?? "")\(s.tables.count) tables · \(s.refs.count) relations\(s.enums.isEmpty ? "" : " · \(s.enums.count) enums")")
                            .font(.headline)
                        Button { reset() } label: { Image(systemName: "arrow.counterclockwise") }.help("Reset layout")
                    }
                    .buttonStyle(.borderless)
                    .padding(12)
                }
                .overlay(alignment: .bottomLeading) {
                    Text("Drag tables (or a group's title) to arrange · point at a column or line to follow a relation")
                        .font(.caption).foregroundStyle(.secondary).padding(14)
                }
            } else if let e = error {
                DiagramError(text: e)
            } else {
                ProgressView()
            }
        }
        .task(id: version) { await load() }
    }

    private func load() async {
        do {
            let s = try JSONDecoder().decode(ErdScene.self, from: try await CanvasCLI.layout(file))
            tables = s.tables.map { $0.rect?.cgRect ?? .zero }
            enums = s.enums.map { $0.rect?.cgRect ?? .zero }
            if let data = FileManager.default.contents(atPath: posFile),
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                let tp = obj["tables"] as? [String: [Double]] ?? [:]
                let ep = obj["enums"] as? [String: [Double]] ?? [:]
                for (i, t) in s.tables.enumerated() { if let p = tp[t.name], p.count == 2 { tables[i].origin = CGPoint(x: p[0], y: p[1]) } }
                for (i, e) in s.enums.enumerated() { if let p = ep[e.name], p.count == 2 { enums[i].origin = CGPoint(x: p[0], y: p[1]) } }
            }
            scene = s
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func save() {
        guard let s = scene, !dragging.isEmpty else { dragging = []; return }
        dragging = []
        var tp: [String: [Double]] = [:], ep: [String: [Double]] = [:]
        for (i, t) in s.tables.enumerated() { tp[t.name] = [tables[i].minX.rounded(), tables[i].minY.rounded()] }
        for (i, e) in s.enums.enumerated() { ep[e.name] = [enums[i].minX.rounded(), enums[i].minY.rounded()] }
        if let data = try? JSONSerialization.data(withJSONObject: ["tables": tp, "enums": ep]) {
            try? data.write(to: URL(fileURLWithPath: posFile))
        }
    }

    private func reset() {
        try? FileManager.default.removeItem(atPath: posFile)
        guard let s = scene else { return }
        tables = s.tables.map { $0.rect?.cgRect ?? .zero }
        enums = s.enums.map { $0.rect?.cgRect ?? .zero }
        fitToken += 1
    }

    private func groupRect(_ g: Int) -> CGRect? {
        guard let s = scene else { return nil }
        let rs = s.groups[g].tables.filter { tables.indices.contains($0) }.map { tables[$0] }
        guard let first = rs.first else { return nil }
        let u = rs.dropFirst().reduce(first) { $0.union($1) }
        return CGRect(x: u.minX - 22, y: u.minY - 52, width: u.width + 44, height: u.height + 74)
    }

    private func bounds() -> CGRect {
        var all = tables + enums
        if let s = scene { all += s.groups.indices.compactMap(groupRect) }
        guard let f = all.first else { return CGRect(x: 0, y: 0, width: 100, height: 100) }
        let u = all.dropFirst().reduce(f) { $0.union($1) }
        return CGRect(x: min(u.minX, 0), y: min(u.minY, 0), width: u.maxX - min(u.minX, 0), height: u.maxY - min(u.minY, 0))
    }

    private func rowY(_ t: Int, _ c: Int) -> CGFloat {
        guard let s = scene else { return 0 }
        return tables[t].minY + s.head + CGFloat(c) * s.row + s.row / 2
    }

    private func anchors(_ k: Int) -> (CGPoint, CGPoint, CGFloat, CGFloat) {
        let r = scene!.refs[k]
        let ra = tables[r.from[0]], rb = tables[r.to[0]]
        let ya = rowY(r.from[0], r.from[1]), yb = rowY(r.to[0], r.to[1])
        if r.from[0] == r.to[0] { return (CGPoint(x: ra.maxX, y: ya), CGPoint(x: rb.maxX, y: yb), 1, -1) }
        if rb.minX >= ra.maxX - 20 { return (CGPoint(x: ra.maxX, y: ya), CGPoint(x: rb.minX, y: yb), 1, 1) }
        if ra.minX >= rb.maxX - 20 { return (CGPoint(x: ra.minX, y: ya), CGPoint(x: rb.maxX, y: yb), -1, -1) }
        return (CGPoint(x: ra.maxX, y: ya), CGPoint(x: rb.maxX, y: yb), 1, -1)
    }

    private func hoverAt(_ p: CGPoint?) {
        guard let p, let s = scene else { hoverTable = nil; hoverRow = nil; hoverRef = nil; return }
        if let i = tables.indices.last(where: { tables[$0].contains(p) }) {
            hoverTable = i
            let c = Int((p.y - tables[i].minY - s.head) / s.row)
            hoverRow = p.y >= tables[i].minY + s.head && s.tables[i].columns.indices.contains(c) ? [i, c] : nil
            hoverRef = nil
            return
        }
        hoverTable = nil; hoverRow = nil
        hoverRef = s.refs.indices.min { a, b in dist(a, p) < dist(b, p) }.flatMap { dist($0, p) < 6 ? $0 : nil }
    }

    private func dist(_ k: Int, _ p: CGPoint) -> CGFloat {
        let (a, b, o, i) = anchors(k)
        return Draw.sCurveDistance(a, b, out: o, in: i, to: p)
    }

    private func grab(_ p: CGPoint) -> Bool {
        if let i = tables.indices.last(where: { tables[$0].contains(p) }) { dragging = [(true, i)]; return true }
        if let i = enums.indices.last(where: { enums[$0].contains(p) }) { dragging = [(false, i)]; return true }
        if let s = scene, let g = s.groups.indices.first(where: { groupRect($0).map { CGRect(x: $0.minX, y: $0.minY, width: $0.width, height: 36).contains(p) } ?? false }) {
            dragging = s.groups[g].tables.map { (true, $0) }
            return true
        }
        return false
    }

    private func move(_ d: CGSize) {
        for (isTable, i) in dragging {
            if isTable { tables[i] = tables[i].offsetBy(dx: d.width, dy: d.height) } else { enums[i] = enums[i].offsetBy(dx: d.width, dy: d.height) }
        }
    }

    private func involved(_ k: Int) -> Bool {
        guard let s = scene else { return false }
        let r = s.refs[k]
        if hoverRef == k { return true }
        if let hr = hoverRow { return r.from == hr || r.to == hr }
        if let t = hoverTable { return r.from[0] == t || r.to[0] == t }
        return false
    }

    private func marker(_ ctx: inout GraphicsContext, _ at: CGPoint, _ dir: CGFloat, _ card: String, _ color: Color) {
        var p = Path()
        if card == "one" {
            p.move(to: CGPoint(x: at.x + dir * 9, y: at.y - 6)); p.addLine(to: CGPoint(x: at.x + dir * 9, y: at.y + 6))
        } else {
            for dy in [-7.0, 0, 7] { p.move(to: CGPoint(x: at.x + dir * 13, y: at.y)); p.addLine(to: CGPoint(x: at.x, y: at.y + dy)) }
        }
        ctx.stroke(p, with: .color(color), lineWidth: 1.4)
    }

    private func draw(_ ctx: inout GraphicsContext, _ s: ErdScene, _ t: DiagramTheme) {
        let focus = hoverTable != nil || hoverRef != nil
        for (g, grp) in s.groups.enumerated() {
            guard let r = groupRect(g) else { continue }
            let c = Color(css: grp.color) ?? t.muted
            let path = Path(roundedRect: r, cornerRadius: 14)
            ctx.fill(path, with: .color(c.opacity(t.dark ? 0.10 : 0.07)))
            ctx.stroke(path, with: .color(c.opacity(0.45)), style: StrokeStyle(lineWidth: 1, dash: [6, 4]))
            Draw.text(&ctx, grp.name, in: CGRect(x: r.minX + 14, y: r.minY + 6, width: r.width - 28, height: 18), size: 11, weight: .semibold, color: c)
        }
        for k in s.refs.indices {
            let (a, b, o, i) = anchors(k)
            let hot = involved(k)
            let color = (hot ? t.accent : t.edge).opacity(focus && !hot ? 0.22 : 1)
            ctx.stroke(Draw.sCurve(a, b, out: o, in: i), with: .color(color), lineWidth: hot ? 2 : 1.4)
            marker(&ctx, a, o, s.refs[k].fromCard, color)
            marker(&ctx, b, -i, s.refs[k].toCard, color)
        }
        for (e, en) in s.enums.enumerated() {
            let r = enums[e]
            var c = ctx
            c.opacity = focus ? 0.5 : 1
            c.fill(Path(roundedRect: r, cornerRadius: 8), with: .color(t.card))
            c.fill(Path(roundedRect: CGRect(x: r.minX, y: r.minY, width: r.width, height: s.head), cornerRadius: 8), with: .color(Color(hex: 0x7A6F9B)))
            Draw.text(&c, "enum  \(en.name)", in: CGRect(x: r.minX + 12, y: r.minY, width: r.width - 24, height: s.head), weight: .semibold, color: .white)
            for (k, v) in en.values.enumerated() {
                Draw.text(&c, v, in: CGRect(x: r.minX + 14, y: r.minY + s.head + CGFloat(k) * s.row, width: r.width - 28, height: s.row), size: 11.5, color: t.fg)
            }
            c.stroke(Path(roundedRect: r.insetBy(dx: 0.5, dy: 0.5), cornerRadius: 8), with: .color(t.border), lineWidth: 1)
        }
        let litTables = Set(s.refs.indices.filter(involved).flatMap { [s.refs[$0].from[0], s.refs[$0].to[0]] })
        for (i, tb) in s.tables.enumerated() {
            let r = tables[i]
            var c = ctx
            c.opacity = !focus || hoverTable == i || litTables.contains(i) ? 1 : 0.4
            c.fill(Path(roundedRect: r.offsetBy(dx: 0, dy: 3), cornerRadius: 8), with: .color(.black.opacity(t.dark ? 0.35 : 0.09)))
            c.fill(Path(roundedRect: r, cornerRadius: 8), with: .color(t.card))
            let hc = Color(css: tb.color) ?? tb.group.flatMap { Color(css: s.groups[$0].color) } ?? (t.dark ? Color(hex: 0x3B5A86) : Color(hex: 0x316896))
            c.fill(Path(roundedRect: CGRect(x: r.minX, y: r.minY, width: r.width, height: s.head), cornerRadius: 8), with: .color(hc))
            c.fill(Path(CGRect(x: r.minX, y: r.minY + s.head - 8, width: r.width, height: 8)), with: .color(hc))
            let parts = tb.name.split(separator: ".", maxSplits: 1).map(String.init)
            let title = parts.count == 2 && parts[0] != "public" ? tb.name : (parts.last ?? tb.name)
            Draw.text(&c, title, in: CGRect(x: r.minX + 12, y: r.minY, width: r.width - 24, height: s.head), weight: .semibold,
                      color: hc.isLight ? Color(hex: 0x1F1E1D) : .white)
            for (ci, col) in tb.columns.enumerated() {
                let y = r.minY + s.head + CGFloat(ci) * s.row
                let hotRow = hoverRow == [i, ci] || s.refs.indices.contains { involved($0) && (s.refs[$0].from == [i, ci] || s.refs[$0].to == [i, ci]) }
                if hotRow { c.fill(Path(CGRect(x: r.minX + 1, y: y, width: r.width - 2, height: s.row)), with: .color(t.accent.opacity(t.dark ? 0.18 : 0.10))) }
                if ci > 0 {
                    var l = Path(); l.move(to: CGPoint(x: r.minX + 8, y: y + 0.5)); l.addLine(to: CGPoint(x: r.maxX - 8, y: y + 0.5))
                    c.stroke(l, with: .color(t.border.opacity(0.6)), lineWidth: 1)
                }
                var x = r.minX + 12
                if col.pk {
                    c.draw(Image(systemName: "key.fill"), in: CGRect(x: x, y: y + s.row / 2 - 6, width: 12, height: 12))
                    x += 18
                }
                let fk = s.refs.contains { $0.from == [i, ci] || $0.to == [i, ci] }
                Draw.text(&c, col.name + (col.notNull ? " *" : ""), in: CGRect(x: x, y: y, width: r.width * 0.55, height: s.row),
                          size: 11.5, weight: col.pk ? .semibold : .regular, color: fk && !col.pk ? t.blue : t.fg)
                let isEnum = s.enums.contains { $0.name == col.type }
                Draw.text(&c, col.type, in: CGRect(x: r.maxX - 12 - r.width * 0.42, y: y, width: r.width * 0.42, height: s.row),
                          alignment: .trailing, size: 11, mono: true, color: isEnum ? Color(hex: 0x8A70C8) : t.muted)
            }
            c.stroke(Path(roundedRect: r.insetBy(dx: 0.5, dy: 0.5), cornerRadius: 8),
                     with: .color(hoverTable == i ? t.accent : t.border), lineWidth: hoverTable == i ? 1.6 : 1)
        }
    }
}

// MARK: - Sequence diagram

struct SequenceDiagramView: View {
    let scene: SeqScene

    private struct Placed { var y: CGFloat; var event: SeqEvent }

    private var xs: [CGFloat] {
        var x: CGFloat = 20, out: [CGFloat] = []
        for p in scene.participants {
            let w = max(CGFloat(p.label.count) * 7.5 + 40, 120)
            out.append(x + w / 2)
            x += w + 40
        }
        return out
    }

    private var layout: (rows: [Placed], height: CGFloat) {
        var y: CGFloat = 70, rows: [Placed] = []
        for e in scene.events {
            let lines = CGFloat(max(1, (e.text ?? "").split(separator: "\n").count))
            switch e.type {
            case "message": y += 16 * lines; rows.append(Placed(y: y, event: e)); y += e.from == e.to ? 46 : 30
            case "note": rows.append(Placed(y: y, event: e)); y += 22 + 16 * lines
            case "start": rows.append(Placed(y: y, event: e)); y += 30
            case "else": rows.append(Placed(y: y, event: e)); y += 28
            default: rows.append(Placed(y: y, event: e)); y += 14
            }
        }
        return (rows, y + 60)
    }

    var body: some View {
        let xs = self.xs
        let (rows, height) = layout
        let width = (xs.last ?? 100) + 120
        ZoomCanvas(contentSize: CGSize(width: width, height: height), minFit: 0.6) { ctx, t in
            draw(&ctx, t, xs, rows, height)
        }
        .overlay(alignment: .topLeading) {
            if !scene.title.isEmpty {
                toolbarCapsule { Text(scene.title).font(.headline) }.padding(12)
            }
        }
    }

    private func box(_ ctx: inout GraphicsContext, _ t: DiagramTheme, _ p: SeqParticipant, _ x: CGFloat, _ y: CGFloat) {
        let w = max(CGFloat(p.label.count) * 7.5 + 28, 100)
        let r = CGRect(x: x - w / 2, y: y, width: w, height: 36)
        ctx.fill(Path(roundedRect: r, cornerRadius: 8), with: .color(t.card))
        ctx.stroke(Path(roundedRect: r, cornerRadius: 8), with: .color(t.border), lineWidth: 1)
        let label = p.actor ? "👤 \(p.label)" : p.label
        ctx.draw(Text(label).font(.system(size: 13, weight: .semibold)).foregroundStyle(t.fg), at: CGPoint(x: r.midX, y: r.midY), anchor: .center)
    }

    private func draw(_ ctx: inout GraphicsContext, _ t: DiagramTheme, _ xs: [CGFloat], _ rows: [Placed], _ height: CGFloat) {
        for (i, x) in xs.enumerated() {
            var life = Path(); life.move(to: CGPoint(x: x, y: 56)); life.addLine(to: CGPoint(x: x, y: height - 56))
            ctx.stroke(life, with: .color(t.border), style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
            box(&ctx, t, scene.participants[i], x, 16)
            box(&ctx, t, scene.participants[i], x, height - 52)
        }
        let left = (xs.first ?? 0) - 70, right = (xs.last ?? 0) + 70
        var frames: [(CGFloat, SeqEvent, Int)] = []
        for row in rows {
            let e = row.event
            switch e.type {
            case "message":
                guard let f = e.from, let to = e.to, xs.indices.contains(f), xs.indices.contains(to) else { continue }
                let color = t.fg.opacity(0.85)
                let style = StrokeStyle(lineWidth: 1.4, dash: e.dashed == true ? [5, 4] : [])
                var label = e.text ?? ""
                if let n = e.number { label = "\(n). " + label }
                if f == to {
                    var p = Path()
                    p.move(to: CGPoint(x: xs[f], y: row.y)); p.addLine(to: CGPoint(x: xs[f] + 40, y: row.y))
                    p.addLine(to: CGPoint(x: xs[f] + 40, y: row.y + 22)); p.addLine(to: CGPoint(x: xs[f] + 2, y: row.y + 22))
                    ctx.stroke(p, with: .color(color), style: style)
                    Draw.arrowHead(&ctx, tip: CGPoint(x: xs[f] + 2, y: row.y + 22), from: CGPoint(x: xs[f] + 20, y: row.y + 22), color: color)
                    ctx.draw(Text(label).font(.system(size: 12)).foregroundStyle(t.fg), at: CGPoint(x: xs[f] + 46, y: row.y + 4), anchor: .leading)
                } else {
                    let a = CGPoint(x: xs[f], y: row.y), b = CGPoint(x: xs[to] + (xs[to] > xs[f] ? -2 : 2), y: row.y)
                    var p = Path(); p.move(to: a); p.addLine(to: b)
                    ctx.stroke(p, with: .color(color), style: style)
                    switch e.head {
                    case "arrow": Draw.arrowHead(&ctx, tip: b, from: a, color: color)
                    case "open":
                        var h = Path()
                        let dir: CGFloat = xs[to] > xs[f] ? -1 : 1
                        h.move(to: CGPoint(x: b.x + dir * 9, y: b.y - 5)); h.addLine(to: b); h.addLine(to: CGPoint(x: b.x + dir * 9, y: b.y + 5))
                        ctx.stroke(h, with: .color(color), lineWidth: 1.4)
                    case "cross":
                        var h = Path()
                        h.move(to: CGPoint(x: b.x - 5, y: b.y - 5)); h.addLine(to: CGPoint(x: b.x + 5, y: b.y + 5))
                        h.move(to: CGPoint(x: b.x + 5, y: b.y - 5)); h.addLine(to: CGPoint(x: b.x - 5, y: b.y + 5))
                        ctx.stroke(h, with: .color(color), lineWidth: 1.6)
                    default: break
                    }
                    ctx.draw(Text(label).font(.system(size: 12)).foregroundStyle(t.fg),
                             at: CGPoint(x: (a.x + b.x) / 2, y: row.y - 4), anchor: .bottom)
                }
            case "note":
                let over = (e.over ?? []).filter { xs.indices.contains($0) }
                guard let first = over.first else { continue }
                let text = e.text ?? ""
                let w = max(CGFloat(text.count) * 7 + 20, 110)
                let x0: CGFloat
                switch e.side {
                case "left": x0 = xs[first] - w - 12
                case "right": x0 = xs[first] + 12
                default:
                    let lo = over.map { xs[$0] }.min()!, hi = over.map { xs[$0] }.max()!
                    x0 = min((lo + hi) / 2 - w / 2, lo - 20)
                }
                let wide = e.side == "over" && over.count > 1 ? max(w, over.map { xs[$0] }.max()! - over.map { xs[$0] }.min()! + 40) : w
                let lines = CGFloat(max(1, text.split(separator: "\n").count))
                let r = CGRect(x: x0, y: row.y, width: wide, height: 8 + 16 * lines)
                ctx.fill(Path(roundedRect: r, cornerRadius: 4), with: .color(Color(hex: 0xFFF4C2).opacity(t.dark ? 0.25 : 1)))
                ctx.stroke(Path(roundedRect: r, cornerRadius: 4), with: .color(Color(hex: 0xD9B84A)), lineWidth: 1)
                ctx.draw(Text(text).font(.system(size: 12)).foregroundStyle(t.fg), at: CGPoint(x: r.midX, y: r.midY), anchor: .center)
            case "start":
                frames.append((row.y, e, frames.count))
            case "else":
                guard let f = frames.last else { continue }
                let inset = CGFloat(f.2) * 8
                var p = Path(); p.move(to: CGPoint(x: left + inset, y: row.y)); p.addLine(to: CGPoint(x: right - inset, y: row.y))
                ctx.stroke(p, with: .color(t.muted), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                if let l = e.label, !l.isEmpty {
                    ctx.draw(Text("[\(l)]").font(.system(size: 11)).foregroundStyle(t.muted), at: CGPoint(x: left + inset + 8, y: row.y + 4), anchor: .topLeading)
                }
            default:
                guard let f = frames.popLast() else { continue }
                let inset = CGFloat(f.2) * 8
                let r = CGRect(x: left + inset, y: f.0, width: right - left - 2 * inset, height: row.y - f.0)
                ctx.stroke(Path(roundedRect: r, cornerRadius: 4), with: .color(t.muted), lineWidth: 1)
                let tag = f.1.kind ?? "loop"
                let tr = CGRect(x: r.minX, y: r.minY, width: CGFloat(tag.count) * 7 + 16, height: 18)
                ctx.fill(Path(tr), with: .color(t.group))
                ctx.draw(Text(tag).font(.system(size: 11, weight: .bold)).foregroundStyle(t.fg), at: CGPoint(x: tr.midX, y: tr.midY), anchor: .center)
                if let l = f.1.label, !l.isEmpty {
                    ctx.draw(Text("[\(l)]").font(.system(size: 11)).foregroundStyle(t.muted), at: CGPoint(x: tr.maxX + 6, y: tr.midY), anchor: .leading)
                }
            }
        }
    }
}
