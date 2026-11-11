import AppKit
import SwiftUI

struct SettingsView: View {
    var body: some View {
        TabView {
            GeneralSettings().tabItem { Label("General", systemImage: "gearshape") }
            AccountsSettings().tabItem { Label("Accounts", systemImage: "person.2") }
            FoldersSettings().tabItem { Label("Folders", systemImage: "folder") }
        }
        .frame(width: 560, height: 420)
    }
}

struct GeneralSettings: View {
    @EnvironmentObject var state: AppState
    @AppStorage(AppAppearance.key) private var appearance = AppAppearance.system.rawValue

    var body: some View {
        Form {
            Section {
                Picker("Appearance", selection: $appearance) {
                    ForEach(AppAppearance.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented)
                .onChange(of: appearance) { _, v in AppAppearance.apply(AppAppearance(rawValue: v) ?? .system) }
            } header: { Text("Window") } footer: {
                Text("System follows macOS Light/Dark mode.").font(.caption).foregroundStyle(.secondary)
            }

            Section {
                Toggle("Move idle Claude sessions when an account nears its limit",
                       isOn: bind(\.auto_switch, "auto_switch") { $0 ? "true" : "false" })
                Text("The conversation resumes on the account with the most room left. Without it, CC Deck only warns you.")
                    .font(.caption).foregroundStyle(.secondary)
            } header: { Text("Auto-switch") }

            Section {
                LabeledContent("5-hour line") {
                    Stepper("\(Int(state.config.threshold))%", value: bind(\.threshold, "threshold") { String($0) },
                            in: 50...100, step: 5)
                }
                LabeledContent("7-day line") {
                    Stepper("\(Int(state.config.weekly_threshold))%",
                            value: bind(\.weekly_threshold, "weekly_threshold") { String($0) }, in: 50...100, step: 1)
                }
                LabeledContent("Idle before moving") {
                    Stepper("\(state.config.idle_secs) s", value: bind(\.idle_secs, "idle_secs") { String($0) },
                            in: 2...120, step: 2)
                }
                LabeledContent("Check usage every") {
                    Picker("", selection: bind(\.poll_secs, "poll_secs") { String($0) }) {
                        Text("1 minute").tag(60)
                        Text("2 minutes").tag(120)
                        Text("5 minutes").tag(300)
                        Text("15 minutes").tag(900)
                    }
                    .labelsHidden()
                    .frame(width: 140)
                }
            } header: { Text("Limit lines") }

            Section {
                LabeledContent("Command-line tool") {
                    Text(DeckCLI.binary).font(.caption.monospaced()).textSelection(.enabled).lineLimit(2)
                }
                Text("In CC Deck terminals, `claude` asks which account to use. `claude-deck` is also on the PATH there.")
                    .font(.caption).foregroundStyle(.secondary)
            } header: { Text("Shell") }
        }
        .formStyle(.grouped)
    }

    private func bind<T>(_ kp: WritableKeyPath<DeckConfig, T>, _ key: String, _ fmt: @escaping (T) -> String) -> Binding<T> {
        Binding(get: { state.config[keyPath: kp] }, set: { v in
            state.config[keyPath: kp] = v
            state.setConfig(key, fmt(v))
        })
    }
}

struct AccountsSettings: View {
    @EnvironmentObject var state: AppState
    @State private var newName = ""
    @State private var confirmRemove: String?
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Form {
            Section {
                ForEach(state.accounts) { acc in
                    HStack(spacing: 10) {
                        Image(systemName: "person.crop.circle.fill").font(.title2).foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack(spacing: 6) {
                                Text(acc.name).fontWeight(.medium)
                                if let p = acc.planLabel { PlanBadge(text: p) }
                            }
                            Text(acc.email ?? acc.stateLabel).font(.caption).foregroundStyle(.secondary)
                        }
                        Spacer()
                        Toggle("Auto", isOn: Binding(get: { !acc.disabled }, set: { on in
                            Task { try? await DeckCLI.setDisabled(acc.name, !on); await state.refresh() }
                        }))
                        .toggleStyle(.switch).controlSize(.mini)
                        .help("Allow auto-switch to move sessions onto this account")
                        if acc.state == "logged_out" || acc.state == "expired" {
                            Button("Log in") { login(acc.name) }.controlSize(.small)
                        }
                        if acc.name != "default" {
                            Button(role: .destructive) { confirmRemove = acc.name } label: { Image(systemName: "trash") }
                                .buttonStyle(.borderless)
                        }
                    }
                    .padding(.vertical, 2)
                }
            } header: { Text("Accounts") } footer: {
                Text("`default` is your usual ~/.claude login. Other accounts are separate Claude Code profiles that share your settings, skills and history.")
                    .font(.caption).foregroundStyle(.secondary)
            }

            Section {
                HStack {
                    TextField("Name, e.g. personal", text: $newName)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit(add)
                    Button("Add and log in", action: add)
                        .disabled(newName.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            } header: { Text("Add an account") }
        }
        .formStyle(.grouped)
        .confirmationDialog("Remove \(confirmRemove ?? "")?", isPresented: Binding(get: { confirmRemove != nil }, set: { if !$0 { confirmRemove = nil } })) {
            Button("Remove", role: .destructive) {
                if let n = confirmRemove {
                    Task {
                        do { try await DeckCLI.remove(n) } catch { state.flash(error.localizedDescription) }
                        await state.refresh()
                    }
                }
            }
        } message: {
            Text("Deletes the profile and its stored login. Shared settings, skills and history stay.")
        }
    }

    private func add() {
        let name = newName.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        Task {
            do {
                try await DeckCLI.add(name)
                newName = ""
                await state.refresh()
                login(name)
            } catch {
                state.flash(error.localizedDescription)
            }
        }
    }

    private func login(_ name: String) {
        state.newTab(cwd: NSHomeDirectory(), run: "claude-deck launch --account \(name)")
        openWindow(id: "main")
        NSApp.activate(ignoringOtherApps: true)
    }
}

struct FoldersSettings: View {
    @EnvironmentObject var state: AppState
    @State private var account = "default"

    var body: some View {
        Form {
            Section {
                if state.config.maps.isEmpty {
                    Text("No folder defaults yet.").foregroundStyle(.secondary)
                }
                ForEach(state.config.maps.sorted(by: { $0.key < $1.key }), id: \.key) { dir, acc in
                    HStack {
                        Image(systemName: "folder").foregroundStyle(.secondary)
                        Text(DeckFormat.shortPath(dir)).lineLimit(1).truncationMode(.head)
                        Spacer()
                        Text(acc).foregroundStyle(.secondary)
                        Button(role: .destructive) {
                            Task { try? await DeckCLI.unmap(dir); await state.refresh() }
                        } label: { Image(systemName: "minus.circle") }
                        .buttonStyle(.borderless)
                    }
                }
            } header: { Text("Folder defaults") } footer: {
                Text("The account picker pre-selects a folder's default. Subfolders inherit it.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            Section {
                HStack {
                    Picker("Account", selection: $account) {
                        ForEach(state.accounts) { Text($0.name).tag($0.name) }
                    }
                    Button("Choose Folder…") { chooseFolder() }
                }
            } header: { Text("Add") }
        }
        .formStyle(.grouped)
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { try? await DeckCLI.map(account, dir: url.path); await state.refresh() }
    }
}
