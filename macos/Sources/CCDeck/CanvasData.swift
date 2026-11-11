import Foundation

// Models decoded from `claude-deck canvas … --json`. The CLI does all parsing and layout; these
// mirror its JSON (src/canvas/*.rs) with optionals so older or partial output still decodes.

struct CanvasMeta: Codable, Hashable {
    var id: String
    var cwd: String
    var project: String
    var title: String
    var agent: String
    var pane: String
    var account: String
    var objective: String
}

struct CanvasState: Codable, Hashable {
    var state: String
    var reason: String
    var at: Int64
}

struct CanvasItem: Codable, Hashable, Identifiable {
    var id: String { name }
    var name: String
    var kind: String
    var mtime: Int64
}

struct CanvasSession: Codable, Hashable, Identifiable {
    var id: String { meta.id }
    var dir: String
    var meta: CanvasMeta
    var state: CanvasState
    var items: [CanvasItem]
    var updated: Int64

    var label: String {
        if !meta.title.isEmpty { return meta.title }
        if !meta.project.isEmpty { return meta.project }
        return String(meta.id.prefix(8))
    }
    var needsYou: Bool { state.state == "waiting" }
    var stateText: String {
        switch state.state {
        case "running": return "running"
        case "waiting":
            return state.reason.isEmpty || state.reason == "question" ? "needs you" : "needs you: \(state.reason)"
        case "idle": return "your turn"
        case "ended": return "ended"
        default: return ""
        }
    }
    func path(_ name: String) -> String { dir + "/content/" + name }
}

struct ObjectiveTask: Codable, Hashable, Identifiable {
    var id: Int { line }
    var line: Int
    var state: String
    var text: String
    var notes: [String]
}

struct Objective: Codable, Hashable, Identifiable {
    var id: String { slug }
    var slug: String
    var title: String
    var status: String
    var tasks: [ObjectiveTask]
    var done: Int { tasks.filter { $0.state == "done" }.count }
}

// MARK: - scenes (`canvas layout`)

typealias JRect = [Double]

extension Array where Element == Double {
    var cgRect: CGRect? { count == 4 ? CGRect(x: self[0], y: self[1], width: self[2], height: self[3]) : nil }
}

struct FlowNode: Codable, Hashable {
    var id: String
    var label: String
    var shape: String
    var group: Bool
    var collapsed: Bool
    var parent: Int?
    var link: String?
    var tip: String?
    var rect: JRect?
    var fill: String?
    var stroke: String?
    var color: String?
    var dashed: Bool?
}

struct FlowEdge: Codable, Hashable {
    var from: Int?
    var to: Int?
    var points: [[Double]]?
    var kind: String
    var arrowStart: Bool
    var arrowEnd: Bool
    var label: String
}

struct FlowScene: Codable {
    var width: Double
    var height: Double
    var nodes: [FlowNode]
    var edges: [FlowEdge]
}

struct LineageColumn: Codable, Hashable {
    var name: String
    var type: String
}

struct LineageNode: Codable, Hashable {
    var name: String
    var kind: String
    var namespace: String
    var short: String
    var expanded: Bool
    var rect: JRect?
    var columns: [LineageColumn]
}

struct LineageScene: Codable {
    var title: String
    var focus: Int?
    var width: Double
    var height: Double
    var head: Double
    var row: Double
    var nodes: [LineageNode]
    var edges: [[Int]]
    var columnEdges: [[Int]]
    var errors: [String]?
}

struct ErdColumn: Codable, Hashable {
    var name: String
    var type: String
    var pk: Bool
    var notNull: Bool
    var unique: Bool
    var increment: Bool
    var `default`: String?
    var note: String?
}

struct ErdTable: Codable, Hashable {
    var name: String
    var alias: String?
    var note: String?
    var color: String?
    var group: Int?
    var rect: JRect?
    var columns: [ErdColumn]
}

struct ErdEnum: Codable, Hashable {
    var name: String
    var values: [String]
    var rect: JRect?
}

struct ErdGroup: Codable, Hashable {
    var name: String
    var color: String?
    var tables: [Int]
}

struct ErdRef: Codable, Hashable {
    var from: [Int]
    var to: [Int]
    var fromCard: String
    var toCard: String
    var name: String?
}

struct ErdScene: Codable {
    var project: String?
    var head: Double
    var row: Double
    var width: Double
    var height: Double
    var tables: [ErdTable]
    var enums: [ErdEnum]
    var groups: [ErdGroup]
    var refs: [ErdRef]
    var errors: [String]?
}

struct SeqParticipant: Codable, Hashable {
    var id: String
    var label: String
    var actor: Bool
}

struct SeqEvent: Codable, Hashable {
    var type: String
    var from: Int?
    var to: Int?
    var text: String?
    var dashed: Bool?
    var head: String?
    var number: Int?
    var side: String?
    var over: [Int]?
    var label: String?
    var kind: String?
}

struct SeqScene: Codable {
    var title: String
    var participants: [SeqParticipant]
    var events: [SeqEvent]
    var errors: [String]?
}

/// Just the `kind` (and error) of any scene or parse output.
struct SceneKind: Codable {
    var kind: String
    var error: String?
}

// MARK: - parse output

struct Finding: Codable, Hashable, Identifiable {
    var id: String { key }
    var key: String
    var file: String
    var line: Int?
    var severity: String
    var message: String
    var fix: String
}

struct ReviewDoc: Codable {
    var title: String
    var findings: [Finding]
}

struct RowStep: Codable, Hashable, Identifiable {
    var id: String { name }
    var name: String
    var rowsIn: Double?
    var out: Double?
    var cols: Double?

    enum CodingKeys: String, CodingKey { case name, rowsIn = "in", out, cols }
}

struct RowsDoc: Codable {
    var title: String
    var steps: [RowStep]
}

struct PathDoc: Codable {
    var path: String
    var line: Int?
}

struct MarkdownDoc: Codable {
    var blocks: [String]
}

struct PlanSub: Codable, Hashable {
    var line: Int
    var summary: String
    var points: Double?
    var assignee: String
    var labels: [String]
    var key: String
}

struct PlanItem: Codable, Hashable, Identifiable {
    var id: Int { n }
    var line: Int
    var n: Int
    var type: String
    var summary: String
    var points: Double?
    var assignee: String
    var labels: [String]
    var key: String
    var description: [String]
    var ac: [String]
    var subtasks: [PlanSub]
    var after: [String]
    var total: Double
}

struct PlanEpic: Codable, Hashable {
    var line: Int?
    var summary: String
    var key: String
    var description: [String]
}

struct PlanBody: Codable {
    var project: String
    var epic: PlanEpic
    var labels: [String]
    var items: [PlanItem]
    var errors: [String]
    var total: Double
}

struct LiveStatus: Codable, Hashable {
    var status: String?
    var category: String?
    var assignee: String?
}

struct PlanDoc: Codable {
    var plan: PlanBody
    var live: [String: LiveStatus]
    var base: String
}

// MARK: - git

struct GitFile: Codable, Hashable, Identifiable {
    var id: String { path }
    var path: String
    var status: String
    var letter: String
    var staged: Bool
    var untracked: Bool
    var added: Int
    var deleted: Int
}

struct GitStatus: Codable {
    var root: String?
    var branch: String?
    var files: [GitFile]?
    var added: Int?
    var deleted: Int?
    var error: String?
}

// MARK: - Jira

struct JiraIssue: Codable, Hashable, Identifiable {
    var id: String { key }
    var key: String
    var summary: String
    var status: String
    var category: String
    var type: String
    var priority: String?
    var assignee: String
    var parent: String?
    var project: String?
    // board cards
    var statusId: String?
    var points: Double?
    var duedate: String?
    var labels: [String]?
    var epic: JiraEpicRef?
    var blocked: Bool?
    var blockedBy: [String]?
    var blocks: [String]?
}

struct JiraEpicRef: Codable, Hashable {
    var key: String
    var summary: String
}

struct JiraLink: Codable, Hashable, Identifiable {
    var id: String
    var type: String
    var label: String
    var key: String
    var summary: String
    var status: String
}

struct JiraComment: Codable, Hashable {
    var author: String
    var created: String
    var text: String
}

struct JiraTransition: Codable, Hashable, Identifiable {
    var id: String
    var name: String
    var to: String
    var toId: String
}

struct JiraDetail: Codable {
    var key: String
    var summary: String
    var status: String
    var category: String
    var type: String
    var assignee: String
    var reporter: String?
    var points: Double?
    var pointsField: Bool?
    var duedate: String?
    var labels: [String]?
    var description: String?
    var url: String?
    var links: [JiraLink]?
    var comments: [JiraComment]?
    var transitions: [JiraTransition]?
    var subtasks: [JiraIssue]?
}

struct JiraColumn: Codable, Hashable, Identifiable {
    var id: String { name }
    var name: String
    var statuses: [String]
    var issues: [JiraIssue]
}

struct JiraSprint: Codable, Hashable {
    var id: Int?
    var name: String
}

struct JiraBoard: Codable {
    var name: String
    var project: String?
    var sprint: JiraSprint?
    var scope: String?
    var columns: [JiraColumn]
}

struct JiraBoardRef: Codable, Hashable, Identifiable {
    var id: Int
    var name: String
    var type: String?
    var project: String?
}

struct JiraEpic: Codable, Hashable, Identifiable {
    var id: String { key }
    var key: String
    var summary: String
    var status: String
    var total: Int
    var done: Int
    var points: Double
    var pointsDone: Double
    var blocked: Int
}

struct JiraUser: Codable {
    var name: String
    var email: String?
}

struct JiraStatusInfo: Codable {
    var configured: Bool
    var base: String?
    var user: JiraUser?
}

// MARK: - CLI calls

enum CanvasCLI {
    static func decode<T: Decodable>(_ t: T.Type, _ args: [String]) async throws -> T {
        let data = try await DeckCLI.run(args)
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw DeckError(message: "Unexpected output from claude-deck \(args.prefix(2).joined(separator: " ")): \(error.localizedDescription)")
        }
    }

    static func text(_ args: [String]) async throws -> String {
        String(data: try await DeckCLI.run(args), encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    static func sessions() async throws -> (Data, [CanvasSession]) {
        let data = try await DeckCLI.run(["canvas", "ls", "--json"])
        return (data, (try? JSONDecoder().decode([CanvasSession].self, from: data)) ?? [])
    }

    static func objectives() async throws -> [Objective] {
        try await decode([Objective].self, ["canvas", "objectives", "--json"])
    }

    static func edit(_ slug: String, _ op: [String: Any]) async throws -> String {
        let json = String(data: try JSONSerialization.data(withJSONObject: op), encoding: .utf8) ?? "{}"
        return try await text(["canvas", "edit", slug, json])
    }

    static func answer(dir: String, screen: String, choices: [String], note: String) async throws {
        var args = ["canvas", "answer", dir, screen, "--note", note]
        for c in choices { args += ["--choice", c] }
        _ = try await DeckCLI.run(args)
    }

    static func kind(of file: String) async throws -> SceneKind {
        try await decode(SceneKind.self, ["canvas", "parse", file])
    }

    static func layout(_ file: String, collapsed: [String]? = nil, expanded: [Int]? = nil) async throws -> Data {
        var args = ["canvas", "layout", file]
        if let c = collapsed { args += ["--collapsed", c.joined(separator: ",")] }
        if let e = expanded { args += ["--expanded", e.map(String.init).joined(separator: ",")] }
        return try await DeckCLI.run(args)
    }

    static func parse<T: Decodable>(_ t: T.Type, _ file: String) async throws -> T {
        try await decode(T.self, ["canvas", "parse", file])
    }

    static func git(_ dir: String) async throws -> GitStatus {
        try await decode(GitStatus.self, ["canvas", "git", dir])
    }

    static func diff(_ dir: String, file: String, untracked: Bool) async throws -> String {
        var args = ["canvas", "git", dir, "--diff", file]
        if untracked { args.append("--untracked") }
        return String(data: try await DeckCLI.run(args), encoding: .utf8) ?? ""
    }

    static func jira<T: Decodable>(_ t: T.Type, _ args: [String]) async throws -> T {
        try await decode(T.self, ["canvas", "jira"] + args + ["--json"])
    }

    /// A Jira write; returns what it did.
    static func jiraDo(_ args: [String]) async throws -> String {
        try await text(["canvas", "jira"] + args)
    }
}

enum CanvasFormat {
    static func ago(_ ms: Int64) -> String {
        let d = Date(timeIntervalSince1970: Double(ms) / 1000)
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .abbreviated
        return f.localizedString(for: d, relativeTo: Date())
    }

    static func icon(_ kind: String) -> String {
        switch kind {
        case "flow": return "point.3.connected.trianglepath.dotted"
        case "lineage": return "arrow.triangle.branch"
        case "erd": return "tablecells"
        case "review": return "checklist"
        case "rows": return "chart.bar.doc.horizontal"
        case "doc", "markdown": return "doc.richtext"
        case "open": return "folder"
        case "plan": return "list.bullet.indent"
        case "html": return "globe"
        default: return "doc"
        }
    }

    static func number(_ v: Double) -> String {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        f.maximumFractionDigits = 0
        return f.string(from: NSNumber(value: v)) ?? "\(Int(v))"
    }
}
