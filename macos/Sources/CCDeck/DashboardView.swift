import Charts
import SwiftUI

struct DashboardView: View {
    @EnvironmentObject var state: AppState
    @State private var stats: DeckStats?
    @State private var days = 14
    @State private var loading = false
    @State private var error: String?

    var body: some View {
        ScrollViewReader { proxy in
        ScrollView {
            Group {
                if let s = stats {
                    DashboardContent(stats: s, days: days)
                        .onAppear {
                            // Development aid for screenshots: CC_DECK_DASH_SCROLL=1 opens scrolled to the end.
                            if ProcessInfo.processInfo.environment["CC_DECK_DASH_SCROLL"] != nil {
                                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { proxy.scrollTo("dash-end", anchor: .bottom) }
                            }
                        }
                } else if let e = error {
                    ContentUnavailableView("Couldn't load statistics", systemImage: "exclamationmark.triangle",
                                           description: Text(e))
                        .padding(.top, 80)
                } else {
                    ProgressView().controlSize(.large).padding(.top, 160)
                }
            }
            .frame(maxWidth: 1180)
            .frame(maxWidth: .infinity)
            Color.clear.frame(height: 1).id("dash-end")
        }
        }
        .navigationTitle("Dashboard")
        .navigationSubtitle(stats.map { "Last \($0.days) days · updated \(Self.time.string(from: Date()))" } ?? "")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Picker("Period", selection: $days) {
                    Text("7D").tag(7)
                    Text("14D").tag(14)
                    Text("30D").tag(30)
                    Text("90D").tag(90)
                }
                .pickerStyle(.segmented)
                .help("Period")
            }
            ToolbarSpacer(.fixed, placement: .primaryAction)
            ToolbarItem(placement: .primaryAction) {
                Button { Task { await load() } } label: { Label("Reload", systemImage: "arrow.clockwise") }
                    .disabled(loading)
            }
        }
        .task(id: days) { await load() }
    }

    private static let time: DateFormatter = { let f = DateFormatter(); f.timeStyle = .short; return f }()

    private func load() async {
        loading = true
        defer { loading = false }
        do {
            stats = try await DeckCLI.stats(days: days)
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }
}

// MARK: - Content

private struct DashboardContent: View {
    let stats: DeckStats
    let days: Int
    @EnvironmentObject var state: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: 28) {
            KPIRow(stats: stats)

            DashSection(title: "Usage limits",
                        subtitle: "How close each account is to its caps. CC Deck switches at the dashed line.") {
                LimitHistoryCard(samples: stats.usage_history)
            }

            DashSection(title: "Spend", subtitle: "Estimated at API list prices from local Claude Code transcripts.") {
                CostChartCard(stats: stats)
                HStack(alignment: .top, spacing: 16) {
                    ModelsCard(models: stats.models).frame(maxHeight: .infinity)
                    ProjectsCard(projects: stats.projects).frame(maxHeight: .infinity)
                }
                .fixedSize(horizontal: false, vertical: true)
            }

            DashSection(title: "Accounts", subtitle: nil) {
                HStack(alignment: .top, spacing: 16) {
                    AccountsTableCard(rows: stats.accounts).frame(maxHeight: .infinity)
                    ActivityCard(events: stats.events).frame(maxHeight: .infinity)
                }
                .fixedSize(horizontal: false, vertical: true)
            }

            Text("Costs are estimates; subscription plans are not billed per token. Sessions from before account tracking are shown as “Before tracking”.")
                .font(.caption).foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 28)
        .padding(.vertical, 24)
    }
}

/// Fixed colour per account, in the order accounts are listed. Unknown buckets are grey.
@MainActor
func accountColor(_ name: String, in accounts: [AccountInfo]) -> Color {
    if let i = accounts.firstIndex(where: { $0.name == name }) { return Theme.series[i % Theme.series.count] }
    return Theme.otherSeries
}

func accountLabel(_ name: String) -> String {
    switch name {
    case "unattributed": return "Before tracking"
    case "other": return "Other"
    default: return name
    }
}

// MARK: - Layout pieces

struct DashSection<Content: View>: View {
    let title: String
    let subtitle: String?
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.title2.weight(.bold))
                if let subtitle { Text(subtitle).font(.callout).foregroundStyle(.secondary) }
            }
            content()
        }
    }
}

struct Card<Content: View>: View {
    var title: String? = nil
    var subtitle: String? = nil
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let title {
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(.headline)
                    if let subtitle { Text(subtitle).font(.subheadline).foregroundStyle(.secondary) }
                }
            }
            content()
        }
        .padding(18)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(.background.secondary, in: .rect(cornerRadius: 18, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(.separator.opacity(0.5), lineWidth: 0.5))
    }
}

// MARK: - KPIs

private struct DayTotal: Identifiable {
    var id: Date { day }
    let day: Date
    var cost: Double
    var tokens: Int
}

private func dayTotals(_ rows: [DayRow]) -> [DayTotal] {
    var map: [Date: DayTotal] = [:]
    for r in rows {
        map[r.day, default: DayTotal(day: r.day, cost: 0, tokens: 0)].cost += r.cost
        map[r.day, default: DayTotal(day: r.day, cost: 0, tokens: 0)].tokens += r.tokens
    }
    return map.values.sorted { $0.day < $1.day }
}

private struct KPIRow: View {
    let stats: DeckStats

    var body: some View {
        let totals = dayTotals(stats.daily)
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 16), count: 4), spacing: 16) {
            KPITile(title: "Estimated cost", symbol: "dollarsign.circle.fill",
                    value: DeckFormat.money(stats.totals.cost),
                    detail: "\(DeckFormat.money(stats.totals.today_cost)) today",
                    spark: totals.map { ($0.day, $0.cost) })
            KPITile(title: "Tokens", symbol: "number.circle.fill",
                    value: DeckFormat.tokens(stats.totals.tokens),
                    detail: "\(DeckFormat.tokens(stats.totals.today_tokens)) today",
                    spark: totals.map { ($0.day, Double($0.tokens)) })
            KPITile(title: "Sessions", symbol: "bubble.left.and.text.bubble.right.fill",
                    value: "\(stats.totals.sessions)",
                    detail: "\(stats.totals.messages) responses", spark: [])
            KPITile(title: "Active time", symbol: "clock.fill",
                    value: DeckFormat.minutes(stats.totals.active_minutes),
                    detail: "across all accounts", spark: [])
        }
    }
}

private struct KPITile: View {
    let title: String
    let symbol: String
    let value: String
    let detail: String
    let spark: [(Date, Double)]

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 6) {
                Label(title, systemImage: symbol)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
                    .symbolRenderingMode(.hierarchical)
                    .lineLimit(1)
                Text(value)
                    .font(.system(size: 30, weight: .semibold, design: .rounded))
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
                    .contentTransition(.numericText())
                Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Group {
                    if spark.count > 1 {
                        Chart(Array(spark.enumerated()), id: \.offset) { _, p in
                            AreaMark(x: .value("Day", p.0), y: .value("Value", p.1))
                                .foregroundStyle(Theme.accent.opacity(0.16).gradient)
                                .interpolationMethod(.monotone)
                            LineMark(x: .value("Day", p.0), y: .value("Value", p.1))
                                .foregroundStyle(Theme.accent)
                                .lineStyle(StrokeStyle(lineWidth: 1.8))
                                .interpolationMethod(.monotone)
                        }
                        .chartXAxis(.hidden)
                        .chartYAxis(.hidden)
                        .accessibilityHidden(true)
                    } else {
                        Color.clear
                    }
                }
                .frame(height: 30)
                .padding(.top, 4)
            }
        }
    }
}

// MARK: - Limits

private struct LimitsList: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            ForEach(state.accounts) { acc in
                HStack(spacing: 14) {
                    UsageRings(five: acc.five?.pct, week: acc.week?.pct,
                               threshold: state.config.threshold, weekly: state.config.weekly_threshold,
                               size: 56, line: 6)
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 6) {
                            Circle().fill(accountColor(acc.name, in: state.accounts)).frame(width: 8, height: 8)
                            Text(acc.name).font(.headline)
                            if let p = acc.planLabel { PlanBadge(text: p) }
                        }
                        if acc.state == "ok" {
                            limitLine("5h", acc.five, state.config.threshold)
                            limitLine("7d", acc.week, state.config.weekly_threshold)
                        } else {
                            Text(acc.stateLabel).font(.callout).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }

    private func limitLine(_ label: String, _ w: UsageWindow?, _ th: Double) -> some View {
        HStack(spacing: 6) {
            Text(label).foregroundStyle(.secondary).frame(width: 20, alignment: .leading)
            Text(w.map { "\(Int($0.pct.rounded()))%" } ?? "–")
                .fontWeight(.semibold)
                .foregroundStyle(w.map { Theme.usageColor($0.pct, line: th) } ?? .secondary)
            if let r = w?.resets_in, !r.isEmpty {
                Text("· resets in \(r)").foregroundStyle(.tertiary)
            }
        }
        .font(.callout.monospacedDigit())
        .lineLimit(1)
        .fixedSize()
    }
}

private struct LimitHistoryCard: View {
    let samples: [UsageSample]
    @EnvironmentObject var state: AppState
    @State private var window = 0
    @State private var hover: Date?

    var body: some View {
        let points = samples.compactMap { s -> (Date, String, Double)? in
            guard let v = window == 0 ? s.five : s.week else { return nil }
            return (s.date, s.account, v)
        }
        let line = window == 0 ? state.config.threshold : state.config.weekly_threshold
        Card {
          HStack(alignment: .top, spacing: 24) {
            VStack(alignment: .leading, spacing: 14) {
                Text("Right now").font(.headline)
                LimitsList()
            }
            .frame(width: 270, alignment: .leading)
            .frame(maxHeight: .infinity, alignment: .center)
            Divider()
            VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("History").font(.headline)
                    Text(window == 0 ? "5-hour window, % used" : "7-day window, % used")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                Spacer()
                Picker("", selection: $window) {
                    Text("5 hours").tag(0)
                    Text("7 days").tag(1)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
            }
            if points.isEmpty {
                EmptyChart(text: "Samples are recorded every few minutes while CC Deck is open.")
            } else {
                Chart {
                    ForEach(Array(points.enumerated()), id: \.offset) { _, p in
                        LineMark(x: .value("Time", p.0), y: .value("Used", p.2))
                            .foregroundStyle(by: .value("Account", p.1))
                            .lineStyle(StrokeStyle(lineWidth: 2, lineCap: .round))
                            .interpolationMethod(.monotone)
                    }
                    RuleMark(y: .value("Switch line", line))
                        .lineStyle(StrokeStyle(lineWidth: 1, dash: [4, 4]))
                        .foregroundStyle(.red.opacity(0.7))
                        .annotation(position: .top, alignment: .trailing) {
                            Text("switch at \(Int(line))%").font(.caption2).foregroundStyle(.red)
                        }
                    if let h = hover {
                        RuleMark(x: .value("Time", h))
                            .foregroundStyle(.secondary.opacity(0.4))
                            .annotation(position: .top, overflowResolution: .init(x: .fit, y: .disabled)) {
                                HoverCard(title: h.formatted(date: .abbreviated, time: .shortened),
                                          rows: nearest(points, to: h).map {
                                              ($0.0, "\(Int($0.1.rounded()))%", accountColor($0.0, in: state.accounts))
                                          })
                            }
                    }
                }
                .chartForegroundStyleScale(domain: state.accounts.map(\.name),
                                           range: state.accounts.map { accountColor($0.name, in: state.accounts) })
                .chartYScale(domain: 0...100)
                .chartYAxis {
                    AxisMarks(position: .trailing, values: [0, 25, 50, 75, 100]) { v in
                        AxisGridLine().foregroundStyle(.separator.opacity(0.5))
                        AxisValueLabel { Text("\(v.as(Int.self) ?? 0)%") }
                    }
                }
                .chartXAxis { AxisMarks { _ in AxisValueLabel() } }
                .chartXSelection(value: $hover)
                .chartLegend(position: .bottom, alignment: .leading)
                .frame(height: 220)
            }
            }
          }
          .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func nearest(_ pts: [(Date, String, Double)], to t: Date) -> [(String, Double)] {
        var best: [String: (TimeInterval, Double)] = [:]
        for p in pts {
            let d = abs(p.0.timeIntervalSince(t))
            if best[p.1].map({ d < $0.0 }) ?? true { best[p.1] = (d, p.2) }
        }
        return best.map { ($0.key, $0.value.1) }.sorted { $0.0 < $1.0 }
    }
}

// MARK: - Spend

private struct CostChartCard: View {
    let stats: DeckStats
    @EnvironmentObject var state: AppState
    @State private var hover: Date?

    var body: some View {
        let totals = dayTotals(stats.daily)
        let avg = totals.isEmpty ? 0 : stats.totals.cost / Double(max(stats.days, 1))
        let rank: (String) -> Int = { name in state.accounts.firstIndex { $0.name == name } ?? 99 }
        let names = Array(Set(stats.daily.map(\.account))).sorted { (rank($0), $0) < (rank($1), $1) }
        Card(title: "Estimated cost per day",
             subtitle: "\(DeckFormat.money(stats.totals.cost)) total · \(DeckFormat.money(avg)) per day on average") {
            if stats.daily.isEmpty {
                EmptyChart(text: "No Claude Code activity in this period.")
            } else {
                Chart {
                    ForEach(stats.daily) { row in
                        BarMark(x: .value("Day", row.day, unit: .day), y: .value("Cost", row.cost), width: .ratio(0.62))
                            .foregroundStyle(by: .value("Account", accountLabel(row.account)))
                            .cornerRadius(3)
                    }
                    RuleMark(y: .value("Average", avg))
                        .lineStyle(StrokeStyle(lineWidth: 1, dash: [3, 3]))
                        .foregroundStyle(.secondary)
                        .annotation(position: .top, alignment: .leading) {
                            Text("avg").font(.caption2).foregroundStyle(.secondary)
                        }
                    if let h = hover, let t = totals.first(where: { Calendar.current.isDate($0.day, inSameDayAs: h) }) {
                        RuleMark(x: .value("Day", t.day, unit: .day))
                            .foregroundStyle(.secondary.opacity(0.15))
                            .lineStyle(StrokeStyle(lineWidth: 28))
                            .annotation(position: .top, overflowResolution: .init(x: .fit, y: .disabled)) {
                                HoverCard(title: t.day.formatted(date: .abbreviated, time: .omitted) + " · " + DeckFormat.money(t.cost),
                                          rows: breakdown(on: t.day))
                            }
                    }
                }
                .chartForegroundStyleScale(domain: names.map(accountLabel),
                                           range: names.map { accountColor($0, in: state.accounts) })
                .chartYAxis {
                    AxisMarks(position: .trailing) { v in
                        AxisGridLine().foregroundStyle(.separator.opacity(0.5))
                        AxisValueLabel { Text(DeckFormat.money(v.as(Double.self) ?? 0)) }
                    }
                }
                .chartXAxis { AxisMarks(values: .stride(by: .day, count: stats.days > 30 ? 7 : stats.days > 14 ? 3 : 1)) { _ in
                    AxisValueLabel(format: .dateTime.day().month(.abbreviated))
                } }
                .chartXSelection(value: $hover)
                .chartLegend(position: .bottom, alignment: .leading)
                .frame(height: 230)
            }
        }
    }

    private func breakdown(on day: Date) -> [(String, String, Color)] {
        var by: [String: Double] = [:]
        for r in stats.daily where Calendar.current.isDate(r.day, inSameDayAs: day) { by[r.account, default: 0] += r.cost }
        return by.sorted { $0.value > $1.value }.map {
            (accountLabel($0.key), DeckFormat.money($0.value), accountColor($0.key, in: state.accounts))
        }
    }
}

private struct ModelsCard: View {
    let models: [DayRow]

    var body: some View {
        Card(title: "By model", subtitle: "Share of estimated cost") {
            if models.isEmpty {
                EmptyChart(text: "No model usage yet.")
            } else {
                let total = max(models.map(\.cost).reduce(0, +), 0.0001)
                VStack(spacing: 12) {
                    ForEach(Array(models.prefix(6).enumerated()), id: \.element.id) { i, m in
                        VStack(alignment: .leading, spacing: 5) {
                            HStack {
                                Text(prettyModel(m.model)).fontWeight(.medium)
                                Spacer()
                                Text(DeckFormat.money(m.cost)).monospacedDigit()
                                Text("\(Int((m.cost / total * 100).rounded()))%")
                                    .monospacedDigit().foregroundStyle(.secondary).frame(width: 40, alignment: .trailing)
                            }
                            .font(.callout)
                            ThinBar(pct: m.cost / total * 100, color: Theme.series[i % Theme.series.count], height: 6)
                            Text("\(DeckFormat.tokens(m.tokens)) tokens · \(m.messages) responses")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }
}

private struct ProjectsCard: View {
    let projects: [ProjectStat]

    var body: some View {
        Card(title: "Top projects", subtitle: "By estimated cost") {
            if projects.isEmpty {
                EmptyChart(text: "No projects yet.")
            } else {
                let maxCost = max(projects.map(\.cost).max() ?? 1, 0.0001)
                VStack(spacing: 10) {
                    ForEach(projects.prefix(7)) { p in
                        HStack(spacing: 10) {
                            Image(systemName: "folder.fill").foregroundStyle(.secondary).font(.caption)
                            VStack(alignment: .leading, spacing: 4) {
                                HStack {
                                    Text((p.project as NSString).lastPathComponent).fontWeight(.medium).lineLimit(1)
                                    Spacer()
                                    Text(DeckFormat.money(p.cost)).monospacedDigit()
                                }
                                .font(.callout)
                                ThinBar(pct: p.cost / maxCost * 100, color: Theme.accent.opacity(0.85))
                            }
                        }
                        .help("\(DeckFormat.shortPath(p.project)) · \(p.sessions) sessions · \(DeckFormat.tokens(p.tokens)) tokens")
                    }
                }
            }
        }
    }
}

// MARK: - Accounts & activity

private struct AccountsTableCard: View {
    let rows: [AccountStat]
    @EnvironmentObject var state: AppState

    var body: some View {
        Card(title: "Per account") {
            Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 10) {
                GridRow {
                    Text("Account"); Text("Sessions"); Text("Tokens"); Text("Cost"); Text("Active")
                }
                .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Divider().gridCellUnsizedAxes(.horizontal)
                ForEach(rows) { a in
                    GridRow {
                        HStack(spacing: 6) {
                            Circle().fill(accountColor(a.account, in: state.accounts)).frame(width: 8, height: 8)
                            Text(accountLabel(a.account)).fontWeight(.medium)
                        }
                        Text("\(a.sessions)")
                        Text(DeckFormat.tokens(a.tokens))
                        Text(DeckFormat.money(a.cost))
                        Text(DeckFormat.minutes(a.active_minutes))
                    }
                    .font(.callout.monospacedDigit())
                }
            }
        }
    }
}

private struct ActivityCard: View {
    let events: [DeckEvent]

    var body: some View {
        let items = events.filter { ["switch", "auto_switch", "limit"].contains($0.kind) }
        Card(title: "Switches & limits") {
            if items.isEmpty {
                Label("No account switches or limit warnings in this period.", systemImage: "checkmark.seal")
                    .font(.callout).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 80)
            } else {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(items.prefix(12).enumerated()), id: \.element.id) { i, e in
                        HStack(alignment: .top, spacing: 12) {
                            VStack(spacing: 0) {
                                Image(systemName: icon(e.kind))
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(e.kind == "limit" ? .orange : Theme.accent)
                                    .frame(width: 24, height: 24)
                                    .background(Circle().fill((e.kind == "limit" ? Color.orange : Theme.accent).opacity(0.14)))
                                if i < min(items.count, 12) - 1 {
                                    Rectangle().fill(.separator).frame(width: 1).frame(maxHeight: .infinity)
                                }
                            }
                            VStack(alignment: .leading, spacing: 2) {
                                Text(title(e)).font(.callout.weight(.medium))
                                Text(e.message).font(.caption).foregroundStyle(.secondary)
                            }
                            .padding(.bottom, 12)
                            Spacer()
                            Text(e.date, format: .relative(presentation: .named))
                                .font(.caption).foregroundStyle(.tertiary)
                        }
                    }
                }
            }
        }
    }

    private func icon(_ kind: String) -> String {
        switch kind {
        case "auto_switch": return "arrow.triangle.2.circlepath"
        case "switch": return "arrow.left.arrow.right"
        default: return "exclamationmark"
        }
    }

    private func title(_ e: DeckEvent) -> String {
        switch e.kind {
        case "auto_switch": return "Auto-switched"
        case "switch": return "Moved to \(e.account ?? "another account")"
        default: return "Near limit"
        }
    }
}

// MARK: - Helpers

struct HoverCard: View {
    let title: String
    let rows: [(String, String, Color)]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.caption.weight(.semibold))
            ForEach(rows, id: \.0) { r in
                HStack(spacing: 6) {
                    Circle().fill(r.2).frame(width: 7, height: 7)
                    Text(r.0).font(.caption)
                    Spacer(minLength: 12)
                    Text(r.1).font(.caption.monospacedDigit().weight(.medium))
                }
            }
        }
        .padding(10)
        .frame(minWidth: 150)
        .glassEffect(.regular, in: .rect(cornerRadius: 12))
    }
}

struct EmptyChart: View {
    let text: String
    var body: some View {
        Text(text).font(.callout).foregroundStyle(.secondary).frame(maxWidth: .infinity, minHeight: 140)
    }
}

func prettyModel(_ id: String) -> String {
    id.replacingOccurrences(of: "claude-", with: "")
        .split(separator: "-")
        .enumerated()
        .map { $0.offset == 0 ? $0.element.capitalized : String($0.element) }
        .joined(separator: " ")
        .replacingOccurrences(of: #"(\d) (\d)"#, with: "$1.$2", options: .regularExpression)
}
