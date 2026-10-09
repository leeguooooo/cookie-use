import SwiftUI

struct SettingsView: View {
    @ObservedObject var prefs: Preferences
    @ObservedObject var updates: UpdateController
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Form {
                Section("Opening an account") {
                    Picker("Return opens", selection: $prefs.openIsolatedByDefault) {
                        Text("My Chrome").tag(false)
                        Text("A separate window").tag(true)
                    }
                    Toggle(isOn: $prefs.cleanSwitch) {
                        Text("Sign out the site’s previous account first")
                        Text("Only that site’s cookies are removed — other sites stay signed in.")
                    }
                }
                Section("Security") {
                    Picker("Ask for Touch ID", selection: $prefs.unlockPolicy) {
                        ForEach(UnlockPolicy.allCases) { Text($0.title).tag($0) }
                    }
                }
                Section("General") {
                    Toggle(isOn: $prefs.hotKeyEnabled) {
                        Text("Quick switcher hotkey")
                        Text("⌥⌘K from anywhere")
                    }
                    Toggle("Launch at login", isOn: $prefs.launchAtLogin)
                }
                Section("Updates") {
                    LabeledContent("Version", value: updates.currentVersion)
                    updateRow
                }
            }
            .formStyle(.grouped)
            HStack {
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent)
            }
            .padding([.horizontal, .bottom], 20)
        }
        .frame(width: 460)
    }

    @ViewBuilder private var updateRow: some View {
        switch updates.state {
        case .idle, .upToDate, .failed:
            HStack {
                switch updates.state {
                case .upToDate: Text("You’re up to date.").foregroundStyle(.secondary)
                case let .failed(message): Text(message).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
                default: EmptyView()
                }
                Spacer()
                Button("Check for Updates") { Task { await updates.check(userInitiated: true) } }
            }
        case .checking:
            HStack { ProgressView().controlSize(.small); Text("Checking…").foregroundStyle(.secondary) }
        case let .available(release):
            HStack {
                Text("Version \(release.version) is available.")
                Spacer()
                Button(release.canInstall ? "Install and Relaunch" : "Open Release Page") { updates.install() }
                    .buttonStyle(.borderedProminent)
            }
        case let .installing(version, progress):
            VStack(alignment: .leading, spacing: 4) {
                Text(progress < 1 ? "Downloading \(version)…" : "Installing \(version)…")
                ProgressView(value: progress)
            }
        }
    }
}
