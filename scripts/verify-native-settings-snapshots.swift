import Foundation
import Vision
import CoreML

// Verify rendered content, so a valid PNG of the wrong page cannot pass the matrix.
func recognizedText(_ url: URL) throws -> String {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["zh-Hans", "en-US"]
    request.usesLanguageCorrection = false
    // Some Intel/Metal drivers stall during OCR. Keep the same recognition
    // quality and assertions while allowing an explicit CPU backend for QA.
    if ProcessInfo.processInfo.environment["AIEYES_OCR_CPU_ONLY"] == "1" {
        for (stage, devices) in try request.supportedComputeStageDevices {
            if let cpu = devices.first(where: { if case .cpu = $0 { return true }; return false }) {
                request.setComputeDevice(cpu, for: stage)
            }
        }
    }
    try VNImageRequestHandler(url: url).perform([request])
    return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined().filter { !$0.isWhitespace }
}

let root = URL(fileURLWithPath: CommandLine.arguments[1])
for scenario in ["empty", "single", "multi", "long", "failure", "capacity"] {
    for theme in ["light", "dark"] {
        print("Native screenshot text: \(scenario) \(theme)")
        let file = root.appendingPathComponent(scenario + "/servers-" + theme + ".png")
        let text = try recognizedText(file)
        precondition(text.contains("台主机") && text.contains("监控设置"), "Server screenshot did not render server controls: " + file.path + "\n" + text)
        precondition(!text.contains("账户额度"), "Server screenshot rendered the Agent overview: " + file.path)
        let guide = try recognizedText(root.appendingPathComponent(scenario + "/setup-guide-" + theme + ".png"))
        precondition(guide.contains("你想用Aieyes做什么") && guide.contains("调整刷新频率"), "Setup guide must show user goals and refresh help")
        let network = try recognizedText(root.appendingPathComponent(scenario + "/setup-network-" + theme + ".png"))
        precondition(network.contains("前置命令") && network.contains("服务器代理配置示例"), "Setup network help must expose the account command and folded proxy examples")
        for page in ["appearance", "desktop", "network", "refresh", "notifications", "updates"] {
            let settingsFile = root.appendingPathComponent(scenario + "/settings-general-" + page + "-" + theme + ".png")
            precondition(FileManager.default.fileExists(atPath: settingsFile.path), "Missing settings subpage: " + settingsFile.path)
        }
    }
}
print("Native snapshots render server controls and cover all six general settings subpages")
