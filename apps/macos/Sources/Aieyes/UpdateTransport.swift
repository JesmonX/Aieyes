import Foundation
import OSLog

/// A per-update-cycle child process shares the core's saved proxy configuration.
/// Only Sparkle's network URLs change; archive bytes and signatures remain intact.
@MainActor final class UpdateTransport {
    private let logger = Logger(subsystem: "app.aieyes.desktop", category: "update-transport")
    private var process: Process?
    private var input: FileHandle?
    private var endpoint: URL?
    private var startup: CheckedContinuation<Void, Error>?
    private var timeout: Task<Void, Never>?
    private(set) var lastFailure: String?
    private(set) var mode = "system"
    var feedURL: URL? { endpoint?.appendingPathComponent("feed.xml") }

    func start(feed: URL) async throws {
        stop()
        lastFailure = nil
        let p = Process(), stdin = Pipe(), stdout = Pipe()
        p.executableURL = URL(fileURLWithPath: ProcessInfo.processInfo.environment["AIEYES_CORE_PATH"]
            ?? Bundle.main.url(forResource: "aieyes-core", withExtension: nil)?.path
            ?? FileManager.default.currentDirectoryPath + "/target/debug/aieyes-core")
        p.arguments = ["--update-transport"]
        var environment = ProcessInfo.processInfo.environment
        if environment["AIEYES_DATA_DIR"] == nil, let path = Bundle.main.object(forInfoDictionaryKey: "AieyesDevelopmentDataDirectory") as? String {
            environment["AIEYES_DATA_DIR"] = path
        }
        p.environment = environment
        p.standardInput = stdin; p.standardOutput = stdout; p.standardError = FileHandle.nullDevice
        let token = UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let data = try JSONSerialization.data(withJSONObject: ["feed": feed.absoluteString, "token": token]) + Data([10])
        try p.run()
        process = p; input = stdin.fileHandleForWriting
        do { try input?.write(contentsOf: data) } catch { stop(); throw error }
        try await withCheckedThrowingContinuation { continuation in
            startup = continuation
            timeout = Task { [weak self] in
                try? await Task.sleep(for: .seconds(5))
                guard !Task.isCancelled else { return }
                self?.stop(error: TransportError.failed("启动更新连接超时，请重试"))
            }
            DispatchQueue.global(qos: .utility).async { [weak self] in
                var buffer = Data()
                let reader = stdout.fileHandleForReading
                while true {
                    let data = reader.availableData
                    if data.isEmpty { break }
                    buffer.append(data)
                    while let end = buffer.firstIndex(of: 10) {
                        let line = Data(buffer[..<end]); buffer.removeSubrange(...end)
                        Task { @MainActor [weak self] in
                            guard self?.process === p else { return }
                            self?.receive(line)
                        }
                    }
                }
                Task { @MainActor [weak self] in
                    guard self?.process === p else { return }
                    self?.stop(error: TransportError.failed("更新连接已断开，请重试"))
                }
            }
        }
    }
    private func receive(_ data: Data) {
        guard let row = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
        if let value = row["endpoint"] as? String, let url = URL(string: value), url.host == "127.0.0.1", url.scheme == "http" {
            endpoint = url; mode = row["mode"] as? String ?? "system"
            timeout?.cancel(); timeout = nil
            let continuation = startup; startup = nil; continuation?.resume()
        } else if let stage = row["stage"] as? String {
            let result = row["result"] as? String ?? "started"
            let elapsed = row["elapsedMs"] as? Int ?? 0
            let attempt = row["attempt"] as? Int ?? 0
            logger.info("stage=\(stage, privacy: .public) mode=\(self.mode, privacy: .public) result=\(result, privacy: .public) attempt=\(attempt) elapsed_ms=\(elapsed)")
            if let message = row["message"] as? String { lastFailure = message }
            else if result == "ok" { lastFailure = nil }
        }
    }
    func archiveURL(for original: URL) -> URL? {
        guard let endpoint, original.scheme == "https", original.user == nil, original.password == nil else { return nil }
        var url = URLComponents(url: endpoint.appendingPathComponent("archive").appendingPathComponent(original.lastPathComponent), resolvingAgainstBaseURL: false)!
        url.queryItems = [URLQueryItem(name: "url", value: original.absoluteString)]
        return url.url
    }
    func stop(error: Error = CancellationError()) {
        timeout?.cancel(); timeout = nil
        let continuation = startup; startup = nil; continuation?.resume(throwing: error)
        try? input?.close(); input = nil
        if let process, process.isRunning { process.terminate() }
        process = nil; endpoint = nil
    }
    deinit { try? input?.close(); if let process, process.isRunning { process.terminate() } }
}
private enum TransportError: LocalizedError {
    case failed(String)
    var errorDescription: String? { if case .failed(let message) = self { return message }; return nil }
}
