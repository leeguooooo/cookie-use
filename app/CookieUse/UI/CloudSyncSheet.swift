import SwiftUI

/// Sync the vault between Macs — through a private GitHub repo (no server,
/// uses the `gh` login) or a CookieCloud server. Newer copy of each account wins.
struct CloudSyncSheet: View {
    @ObservedObject var model: AppModel
    @ObservedObject var prefs: Preferences
    @Environment(\.dismiss) private var dismiss

    enum Backend: Hashable { case github, cookiecloud }

    @State private var backend: Backend = .github
    @State private var joining = false
    // GitHub
    @State private var ghLogin: String?
    @State private var ghChecked = false
    @State private var repo = ""
    @State private var create = true
    // CookieCloud
    @State private var endpoint = ""
    @State private var uuid = ""
    @State private var legacy = false
    @State private var browserCompat = true

    @State private var password = ""
    @State private var secret: CloudSecret?
    @State private var busy = false
    @State private var error: String?

    private var status: CloudStatus? { model.cloud }
    private var configured: Bool { status?.configured == true }

    private var canSetUp: Bool {
        guard !busy else { return false }
        if joining, password.isEmpty { return false }
        switch backend {
        case .github: return repo.split(separator: "/").count == 2 && ghLogin != nil
        case .cookiecloud: return endpoint.hasPrefix("http") && (!joining || !uuid.isEmpty)
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Sync between Macs", systemImage: "arrow.triangle.2.circlepath.icloud").font(.title2.weight(.semibold))
            Text("Keep the same logins on all your Macs. Everything is encrypted on this Mac (argon2id + AES-GCM) before it leaves; the password never does.")
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
                    Button(joining ? "Join" : "Turn on sync") { setup() }
                        .buttonStyle(.borderedProminent).controlSize(.large)
                        .keyboardShortcut(.defaultAction).disabled(!canSetUp)
                }
            }
        }
        .padding(24)
        .frame(width: 520)
        .task {
            await model.loadCloud()
            ghLogin = await CLIBridge.shared.githubLogin()
            ghChecked = true
            if repo.isEmpty, let ghLogin { repo = "\(ghLogin)/cookie-use-sync" }
            if ghLogin == nil { backend = .cookiecloud }
        }
    }

    // MARK: Not configured

    private var setupView: some View {
        VStack(alignment: .leading, spacing: 12) {
            Picker("Sync through", selection: $backend) {
                Text("A private GitHub repo").tag(Backend.github)
                Text("A CookieCloud server").tag(Backend.cookiecloud)
            }
            .pickerStyle(.segmented)

            Picker("", selection: $joining) {
                Text("This is my first Mac").tag(false)
                Text("Join — sync is already on another Mac").tag(true)
            }
            .pickerStyle(.radioGroup).labelsHidden()

            switch backend {
            case .github: githubFields
            case .cookiecloud: cookieCloudFields
            }

            if joining {
                SecureField("Password from your other Mac", text: $password).textFieldStyle(.roundedBorder)
                Text("On that Mac: CookieUse › Sync between Macs › Show password.").font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private var githubFields: some View {
        if ghChecked, ghLogin == nil {
            VStack(alignment: .leading, spacing: 6) {
                Text("Needs the GitHub CLI, signed in:").font(.callout)
                commandRow("brew install gh && gh auth login")
                Button("Check again") { Task { ghLogin = await CLIBridge.shared.githubLogin(); if let l = ghLogin, repo.isEmpty { repo = "\(l)/cookie-use-sync" } } }
            }
            .infoCard()
        } else {
            VStack(alignment: .leading, spacing: 6) {
                TextField("owner/repo", text: $repo).textFieldStyle(.roundedBorder)
                if !joining { Toggle("Create it as a private repo if it doesn’t exist", isOn: $create) }
                Text("No server to run. One encrypted file is committed through your `gh` login (\(ghLogin ?? "…")); public repos are refused.")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var cookieCloudFields: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField("Server URL — https://cookiecloud.example.com", text: $endpoint).textFieldStyle(.roundedBorder)
            if joining { TextField("uuid", text: $uuid).textFieldStyle(.roundedBorder) }
            Text("Your own (`docker run -p 8088:8088 easychen/cookiecloud`) or a public one. Works with the CookieCloud browser extension too.")
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            DisclosureGroup("Advanced") {
                VStack(alignment: .leading, spacing: 6) {
                    Toggle("Legacy encryption (older CookieCloud extensions)", isOn: $legacy)
                    Toggle("Also publish the latest login per site for the CookieCloud extension", isOn: $browserCompat)
                }
                .padding(.top, 4)
            }
            .font(.callout)
        }
    }

    // MARK: Configured

    private var connectedView: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                fact("Syncing via", status?.whereText ?? "")
                fact("Last sync", status?.lastPull.flatMap(ISO8601.parse).map { $0.relative } ?? "never")
                if let secret {
                    if status?.backend != "github" { fact("uuid", secret.uuid, copy: true) }
                    fact("password", secret.password, copy: true)
                } else {
                    Button("Show password (to add another Mac)") { Task { secret = try? await CLIBridge.shared.cloudSecret() } }
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
            Text(key).foregroundStyle(.secondary).frame(width: 84, alignment: .leading)
            Text(value).textSelection(.enabled).font(copy ? .callout.monospaced() : .callout).lineLimit(1)
            Spacer(minLength: 0)
            if copy {
                Button { Clipboard.copy(value) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless)
            }
        }
    }

    private func commandRow(_ cmd: String) -> some View {
        HStack {
            Text(cmd).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
            Spacer()
            Button { Clipboard.copy(cmd) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless)
        }
        .insetField()
    }

    // MARK: Actions

    private func setup() {
        busy = true
        error = nil
        Task {
            do {
                switch backend {
                case .github:
                    secret = try await CLIBridge.shared.cloudSetupGitHub(
                        repo: repo.trimmingCharacters(in: .whitespaces), create: create && !joining,
                        password: joining ? password : nil)
                case .cookiecloud:
                    secret = try await CLIBridge.shared.cloudSetup(
                        endpoint: endpoint.trimmingCharacters(in: .whitespaces),
                        uuid: joining ? uuid.nilIfBlank : nil,
                        password: joining ? password.nilIfBlank : nil,
                        crypto: legacy ? "legacy" : "aes-128-cbc-fixed",
                        browserCompat: browserCompat)
                }
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
