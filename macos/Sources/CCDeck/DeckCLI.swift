import Foundation

// MARK: - Models decoded from `claude-deck state` / `claude-deck stats`

struct UsageWindow: Codable, Hashable {
    let name: String
    let pct: Double
    let resets_at: String?
    let resets_in: String
}

struct AccountInfo: Codable, Identifiable, Hashable {
    var id: String { name }
    let name: String
    let email: String?
    let account_uuid: String?
    let plan: String?
    let disabled: Bool
    let state: String
    let error: String?
    let five: UsageWindow?
    let week: UsageWindow?
    let others: [UsageWindow]
    let over: Bool
    let pressure: Double?

    var planLabel: String? { plan.map { $0.prefix(1).uppercased() + $0.dropFirst() } }
    var isUsable: Bool { state == "ok" && !over }
    var stateLabel: String {
        switch state {
        case "ok": return "Ready"
        case "loading": return "Checking usage…"
        case "logged_out": return "Not logged in"
        case "expired": return "Refreshes on next use"
        default: return "Couldn't read usage"
        }
    }
}

struct DeckConfig: Codable, Equatable {
    var threshold: Double
    var weekly_threshold: Double
    var auto_switch: Bool
    var idle_secs: Int
    var cooldown_secs: Int
    var poll_secs: Int
    var maps: [String: String]

    /// The account mapped to `dir` or its nearest mapped ancestor.
    func account(for dir: String) -> String? {
        var url = URL(fileURLWithPath: dir).standardizedFileURL.resolvingSymlinksInPath()
        while true {
            if let a = maps[url.path] { return a }
            let parent = url.deletingLastPathComponent()
            if parent.path == url.path { return nil }
            url = parent
        }
    }
}

struct DeckState: Codable {
    let accounts: [AccountInfo]
    let config: DeckConfig
}

struct StatTotals: Codable {
    let tokens: Int
    let cost: Double
    let sessions: Int
    let messages: Int
    let active_minutes: Double
    let today_cost: Double
    let today_tokens: Int
}

struct DayRow: Codable, Identifiable {
    var id: String { "\(date)|\(account)|\(model)" }
    let date: String
    let account: String
    let model: String
    let input: Int
    let output: Int
    let cache_write: Int
    let cache_read: Int
    let tokens: Int
    let cost: Double
    let messages: Int

    var day: Date { DeckFormat.day.date(from: date) ?? .distantPast }
}

struct AccountStat: Codable, Identifiable {
    var id: String { account }
    let account: String
    let sessions: Int
    let messages: Int
    let tokens: Int
    let cost: Double
    let active_minutes: Double
}

struct ProjectStat: Codable, Identifiable {
    var id: String { project }
    let project: String
    let sessions: Int
    let tokens: Int
    let cost: Double
    let last_used: String
}

struct UsageSample: Codable, Identifiable {
    var id: String { "\(ts)|\(account)" }
    let ts: String
    let account: String
    let five: Double?
    let week: Double?
    var date: Date { DeckFormat.parseISO(ts) ?? .distantPast }
}

struct DeckEvent: Codable, Identifiable {
    var id: String { "\(ts)|\(kind)|\(message)" }
    let ts: String
    let kind: String
    let account: String?
    let message: String
    var date: Date { DeckFormat.parseISO(ts) ?? .distantPast }
}

struct DeckStats: Codable {
    let generated_at: String
    let days: Int
    let totals: StatTotals
    let daily: [DayRow]
    let accounts: [AccountStat]
    let projects: [ProjectStat]
    let models: [DayRow]
    let usage_history: [UsageSample]
    let events: [DeckEvent]
}

enum DeckFormat {
    static let day: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd"
        f.locale = Locale(identifier: "en_US_POSIX")
        return f
    }()
    static let isoFrac: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()
    static let iso = ISO8601DateFormatter()
    static func parseISO(_ s: String) -> Date? { isoFrac.date(from: s) ?? iso.date(from: s) }

    static func tokens(_ n: Int) -> String {
        let d = Double(n)
        if d >= 1_000_000_000 { return String(format: "%.1fB", d / 1e9) }
        if d >= 1_000_000 { return String(format: "%.1fM", d / 1e6) }
        if d >= 1_000 { return String(format: "%.1fK", d / 1e3) }
        return "\(n)"
    }
    static func money(_ v: Double) -> String { v >= 100 ? String(format: "$%.0f", v) : String(format: "$%.2f", v) }
    static func minutes(_ m: Double) -> String {
        let h = Int(m) / 60, mm = Int(m) % 60
        return h > 0 ? "\(h)h \(mm)m" : "\(mm)m"
    }
    static func shortPath(_ p: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        return p.hasPrefix(home) ? "~" + p.dropFirst(home.count) : p
    }
}

// MARK: - Calling the Rust CLI

struct DeckError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

enum DeckCLI {
    /// The CLI bundled in the app, or a development build.
    static var binary: String {
        if let p = Bundle.main.resourceURL?.appendingPathComponent("bin/claude-deck").path,
           FileManager.default.isExecutableFile(atPath: p) { return p }
        if let p = ProcessInfo.processInfo.environment["CC_DECK_BIN"] { return p }
        let cargo = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".cargo/bin/claude-deck").path
        return cargo
    }

    static var binDir: String { (binary as NSString).deletingLastPathComponent }

    @discardableResult
    static func run(_ args: [String]) async throws -> Data {
        try await withCheckedThrowingContinuation { cont in
            DispatchQueue.global(qos: .userInitiated).async {
                let p = Process()
                p.executableURL = URL(fileURLWithPath: binary)
                p.arguments = args
                let out = Pipe(), err = Pipe()
                p.standardOutput = out
                p.standardError = err
                p.standardInput = FileHandle.nullDevice
                do {
                    try p.run()
                    let data = out.fileHandleForReading.readDataToEndOfFile()
                    let errData = err.fileHandleForReading.readDataToEndOfFile()
                    p.waitUntilExit()
                    if p.terminationStatus == 0 {
                        cont.resume(returning: data)
                    } else {
                        let msg = String(data: errData, encoding: .utf8)?
                            .trimmingCharacters(in: .whitespacesAndNewlines)
                            .replacingOccurrences(of: "Error: ", with: "") ?? "claude-deck failed"
                        cont.resume(throwing: DeckError(message: msg))
                    }
                } catch {
                    cont.resume(throwing: error)
                }
            }
        }
    }

    static func state(record: Bool) async throws -> DeckState {
        let data = try await run(record ? ["state", "--record"] : ["state"])
        return try JSONDecoder().decode(DeckState.self, from: data)
    }

    static func stats(days: Int) async throws -> DeckStats {
        let data = try await run(["stats", "--days", String(days)])
        return try JSONDecoder().decode(DeckStats.self, from: data)
    }

    static func set(_ key: String, _ value: String) async throws { try await run(["config", key, value]) }
    static func map(_ account: String, dir: String) async throws { try await run(["map", account, dir]) }
    static func unmap(_ dir: String) async throws { try await run(["unmap", dir]) }
    static func add(_ name: String) async throws { try await run(["add", name, "--no-login"]) }
    static func remove(_ name: String) async throws { try await run(["rm", name]) }
    static func setDisabled(_ name: String, _ disabled: Bool) async throws {
        try await run([disabled ? "disable" : "enable", name])
    }
    static func event(_ kind: String, _ message: String, account: String?) async {
        var args = ["event", kind, message]
        if let a = account { args += ["--account", a] }
        _ = try? await run(args)
    }
}
