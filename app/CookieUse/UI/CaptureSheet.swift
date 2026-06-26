import SwiftUI

/// Capture a logged-in session from a Chrome profile into the vault.
struct CaptureSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    @State private var profile = "auto"
    @State private var site = ""
    @State private var accountID = ""
    @State private var label = ""
    @State private var hint = ""
    @State private var withLocalStorage = false
    @State private var busy = false

    private var trimmedProfile: String { profile.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var trimmedSite: String { site.trimmingCharacters(in: .whitespacesAndNewlines) }

    private var canSubmit: Bool {
        !busy && !trimmedProfile.isEmpty && !trimmedSite.isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Capture session", systemImage: "plus.circle")
                .font(.title2.weight(.semibold))

            VStack(alignment: .leading, spacing: 12) {
                field(title: "From Chrome profile",
                      help: "The profile name, or \"auto\" to detect.") {
                    TextField("auto", text: $profile)
                        .textFieldStyle(.roundedBorder)
                }

                field(title: "Site(s)",
                      help: "A comma-separated list captures one account across several domains.") {
                    TextField("chatgpt.com,openai.com", text: $site)
                        .textFieldStyle(.roundedBorder)
                }

                field(title: "Account id", help: nil) {
                    TextField("auto-generated", text: $accountID)
                        .textFieldStyle(.roundedBorder)
                }

                field(title: "Label", help: nil) {
                    TextField("optional", text: $label)
                        .textFieldStyle(.roundedBorder)
                }

                field(title: "Hint", help: nil) {
                    TextField("optional", text: $hint)
                        .textFieldStyle(.roundedBorder)
                }

                Toggle("Also capture localStorage", isOn: $withLocalStorage)
            }

            if let error = model.lastError, !error.isEmpty {
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 10) {
                if busy {
                    ProgressView()
                        .controlSize(.small)
                    Text("Capturing…")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Cancel") { dismiss() }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .disabled(busy)

                Button("Capture") { capture() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSubmit)
            }
        }
        .padding(24)
        .frame(width: 440)
    }

    @ViewBuilder
    private func field<Content: View>(title: String, help: String?,
                                      @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(.headline)
            content()
            if let help {
                Text(help)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func capture() {
        guard canSubmit else { return }
        busy = true
        let id = optional(accountID)
        let lbl = optional(label)
        let hnt = optional(hint)
        let profileValue = trimmedProfile
        let siteValue = trimmedSite
        let captureLocal = withLocalStorage
        Task {
            let ok = await model.capture(fromProfile: profileValue,
                                         site: siteValue,
                                         id: id,
                                         label: lbl,
                                         hint: hnt,
                                         withLocalStorage: captureLocal)
            busy = false
            if ok { dismiss() }
        }
    }

    private func optional(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
