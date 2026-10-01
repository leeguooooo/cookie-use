import SwiftUI

struct SettingsView: View {
    @ObservedObject var prefs: Preferences
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
}
