import Foundation
import Vision

// Verify rendered content, so a valid PNG of the wrong page cannot pass the matrix.
func recognizedText(_ url: URL) throws -> String {
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.recognitionLanguages = ["zh-Hans", "en-US"]
    request.usesLanguageCorrection = false
    try VNImageRequestHandler(url: url).perform([request])
    return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined().filter { !$0.isWhitespace }
}

let root = URL(fileURLWithPath: CommandLine.arguments[1])
for scenario in ["empty", "single", "multi", "long", "failure", "capacity"] {
    for theme in ["light", "dark"] {
        let file = root.appendingPathComponent(scenario + "/servers-" + theme + ".png")
        let text = try recognizedText(file)
        precondition(text.contains("台主机") && text.contains("监控设置"), "Server screenshot did not render server controls: " + file.path + "\n" + text)
        precondition(!text.contains("实时账户限额"), "Server screenshot rendered the Agent overview: " + file.path)
        for page in ["appearance", "desktop", "network", "refresh", "updates"] {
            let settingsFile = root.appendingPathComponent(scenario + "/settings-general-" + page + "-" + theme + ".png")
            precondition(FileManager.default.fileExists(atPath: settingsFile.path), "Missing settings subpage: " + settingsFile.path)
        }
    }
}
print("Native snapshots render server controls and cover all five general settings subpages")
