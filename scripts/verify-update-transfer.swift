import Foundation
import CryptoKit

/// Opt-in live read/download check. Uses a temporary config supplied by the runner;
/// never invokes the Sparkle installer or changes the installed application.
@main struct VerifyUpdateTransfer {
    @MainActor static func main() async throws {
        let arguments = CommandLine.arguments
        let info = try PropertyListSerialization.propertyList(from: Data(contentsOf: URL(fileURLWithPath: arguments[1])), format: nil) as! [String: Any]
        let publicKey = try Curve25519.Signing.PublicKey(rawRepresentation: Data(base64Encoded: info["SUPublicEDKey"] as! String)!)
        let feed = URL(string: info["SUFeedURL"] as! String)!
        let transport = UpdateTransport()
        defer { transport.stop() }
        try await transport.start(feed: feed)
        let session = URLSession(configuration: .ephemeral)
        defer { session.invalidateAndCancel() }
        let started = Date()
        let (data, response) = try await session.data(from: transport.feedURL!)
        precondition((response as! HTTPURLResponse).statusCode == 200)
        let feedTime = Date().timeIntervalSince(started)
        let document = try XMLDocument(data: data)
        let enclosure = try document.nodes(forXPath: "//enclosure").first as! XMLElement
        let original = URL(string: enclosure.attribute(forName: "url")!.stringValue!)!
        let signature = Data(base64Encoded: enclosure.attribute(forName: "sparkle:edSignature")!.stringValue!)!
        let length = Int(enclosure.attribute(forName: "length")!.stringValue!)!
        let archive = transport.archiveURL(for: original)!
        precondition(archive.pathExtension == original.pathExtension)
        let downloadStart = Date()
        let (temporary, downloadResponse) = try await session.download(from: archive)
        defer { try? FileManager.default.removeItem(at: temporary) }
        precondition((downloadResponse as! HTTPURLResponse).statusCode == 200)
        let bytes = try Data(contentsOf: temporary, options: .mappedIfSafe)
        precondition(bytes.count == length && publicKey.isValidSignature(signature, for: bytes))
        var tampered = bytes; tampered[0] ^= 1
        precondition(!publicKey.isValidSignature(signature, for: tampered))
        let result: [String: Any] = ["mode": transport.mode, "feedSeconds": feedTime, "archiveSeconds": Date().timeIntervalSince(downloadStart), "archiveBytes": bytes.count, "archiveSignature": "verified", "tamperedArchive": "rejected", "installed": false]
        print(String(data: try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys, .prettyPrinted]), encoding: .utf8)!)
    }
}
