import SwiftUI

/// CookieCloud-compatible sync: set up a server once, then every Mac with the
/// same uuid + password shares the vault (newer copy of each account wins).
struct CloudSyncSheet: View {
    @ObservedObject var model: AppModel
    @ObservedObject var prefs: Preferences
    @Environment(\.dismiss) private var dismiss

    @State private var endpoint = ""
    @State private var uuid = ""
    @State private var password = ""
    @State private var legacy = false
    @State private var browserCompat = true
    @State private var joining = false
    @State private var secret: CloudSecret?
    @State private var busy = false
    @State private var error: String?

    private var status: CloudStatus? { model.cloud }
    private var configured: Bool { status?.configured == true }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Cloud sync", systemImage: "arrow.triangle.2.circlepath.icloud").font(.title2.weight(.semibold))
            Text("Keep the same logins on all your Macs through a CookieCloud server — your own (`docker run -p 8088:8088 easychen/cookiecloud`) or a public one. Everything is encrypted on this Mac before upload.")
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)

            if configured { connectedView } else { setupView }

            if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill").font(.callout).foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 10) {
                if busy { ProgressView().controlSize(.small) }
                Spacer()
                Button("Done") { dismiss() }.controlSize(.large).keyboardShortcut(configured ? .defaultAction : .cancelAction)
                if !configured {
                    Button(joining ? "Join" : "Set up") { setup() }
                        .buttonStyle(.borderedProminent).controlSize(.large)
                        .keyboardShortcut(.defaultAction)
                        .disabled(busy || !endpoint.hasPrefix("http") || (joining && (uuid.isEmpty || password.isEmpty)))
                }
            }
        }
        .padding(24)
        .frame(width: 500)
        .task { await model.loadCloud() }
    }

    private var setupView: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("Server URL — https://cookiecloud.example.com", text: $endpoint).textFieldStyle(.roundedBorder)
            Picker("", selection: $joining) {
                Text("First Mac — create credentials").tag(false)
                Text("Join — I have a uuid and password").tag(true)
            }
            .pickerStyle(.radioGroup).labelsHidden()
            if joining {
                TextField("uuid", text: $uuid).textFieldStyle(.roundedBorder)
                SecureField("password", text: $password).textFieldStyle(.roundedBorder)
            }
            DisclosureGroup("Advanced") {
                VStack(alignment: .leading, spacing: 6) {
                    Toggle("Legacy encryption (older CookieCloud extensions)", isOn: $legacy)
                    Toggle("Also publish the latest login per site for the CookieCloud browser extension", isOn: $browserCompat)
                }
                .padding(.top, 4)
            }
            .font(.callout)
        }
    }

    private var connectedView: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                fact("Server", status?.endpoint ?? "")
                fact("Last sync", status?.lastPull.flatMap(ISO8601.parse).map { $0.relative } ?? "never")
                if let secret {
                    fact("uuid", secret.uuid, copy: true)
                    fact("password", secret.password, copy: true)
                } else {
                    Button("Show uuid and password (to add another Mac)") { Task { secret = try? await CLIBridge.shared.cloudSecret() } }
                        .buttonStyle(.link)
                }
            }
            .font(.callout)
            .infoCard()

            Picker("Sync automatically", selection: $prefs.syncInterval) {
                Text("Off").tag(0)
                Text("Every 5 minutes").tag(5)
                Text("Every 15 minutes").tag(15)
                Text("Every hour").tag(60)
            }

            HStack {
                Button { Task { await sync() } } label: { Label("Sync now", systemImage: "arrow.triangle.2.circlepath") }
                    .buttonStyle(.borderedProminent).disabled(busy)
                Spacer()
                Button("Turn off", role: .destructive) { Task { await disconnect() } }.disabled(busy)
            }
        }
    }

    private func fact(_ key: String, _ value: String, copy: Bool = false) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(key).foregroundStyle(.secondary).frame(width: 72, alignment: .leading)
            Text(value).textSelection(.enabled).font(copy ? .callout.monospaced() : .callout).lineLimit(1)
            Spacer(minLength: 0)
            if copy {
                Button { Clipboard.copy(value) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless)
            }
        }
    }

    private func setup() {
        busy = true
        error = nil
        Task {
            do {
                secret = try await CLIBridge.shared.cloudSetup(
                    endpoint: endpoint.trimmingCharacters(in: .whitespaces),
                    uuid: joining ? uuid.nilIfBlank : nil,
                    password: joining ? password.nilIfBlank : nil,
                    crypto: legacy ? "legacy" : "aes-128-cbc-fixed",
                    browserCompat: browserCompat)
                await model.loadCloud()
                await sync()
                if prefs.syncInterval == 0 { prefs.syncInterval = 15 }
            } catch { self.error = error.localizedDescription }
            busy = false
        }
    }

    private func sync() async {
        busy = true
        error = nil
        if let failure = await model.syncNow() { error = failure }
        busy = false
    }

    private func disconnect() async {
        do {
            try await CLIBridge.shared.cloudDisconnect()
            prefs.syncInterval = 0
            secret = nil
            await model.loadCloud()
        } catch { self.error = error.localizedDescription }
    }
}
