import Foundation

/// UI copy only. Identifiers, paths, credentials and user-authored text stay unchanged.
enum AieyesLocale {
    static var language: String {
        normalize(ProcessInfo.processInfo.environment["AIEYES_LANGUAGE"] ?? UserDefaults.standard.string(forKey: "aieyes.language"))
    }
    static var locale: Locale { Locale(identifier: language == "en" ? "en_US" : "zh_CN") }
    static func month(_ number: Int) -> String { language == "en" ? ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][min(11, max(0, number - 1))] : "\(number)月" }
    static func normalize(_ language: String?) -> String { language == "en" ? "en" : "zh-CN" }
    static func apply(_ language: String?) {
        let next = normalize(language)
        if UserDefaults.standard.string(forKey: "aieyes.language") != next {
            UserDefaults.standard.set(next, forKey: "aieyes.language")
        }
    }
    private struct Pattern {
        let expression: NSRegularExpression
        let target: String
        let slots: [String]
        let specificity: Int
    }
    private static let parameter = try! NSRegularExpression(pattern: #"\{([0-9]+)\}"#)
    private static let reverse: [String: String] = LocalizationCatalog.english.keys.sorted().reduce(into: [:]) { $0[LocalizationCatalog.english[$1]!] = $1 }
    private static func makePatterns(_ catalog: [String: String]) -> [Pattern] { catalog.compactMap { source, target in
        let range = NSRange(source.startIndex..., in: source)
        let matches = parameter.matches(in: source, range: range)
        guard !matches.isEmpty else { return nil }
        var regex = "^", end = source.startIndex
        var slots: [String] = []
        for match in matches {
            guard let whole = Range(match.range, in: source), let slot = Range(match.range(at: 1), in: source) else { continue }
            regex += NSRegularExpression.escapedPattern(for: String(source[end..<whole.lowerBound])) + "([\\s\\S]*?)"
            slots.append(String(source[slot])); end = whole.upperBound
        }
        regex += NSRegularExpression.escapedPattern(for: String(source[end...])) + "$"
        guard let expression = try? NSRegularExpression(pattern: regex) else { return nil }
        return Pattern(expression: expression, target: target, slots: slots, specificity: source.count - matches.reduce(0) { $0 + $1.range.length })
    }.sorted { $0.specificity > $1.specificity } }
    private static let patterns = makePatterns(LocalizationCatalog.english)
    private static let reversePatterns = makePatterns(reverse)
    private static let cache = NSCache<NSString, NSString>()
    static func message(_ source: String) -> String {
        let english = language == "en"
        if let exact = (english ? LocalizationCatalog.english : reverse)[source] { return exact }
        let hasChinese = source.range(of: #"\p{Han}"#, options: .regularExpression) != nil
        if english && !hasChinese { return source }
        if !english && LocalizationCatalog.english[source] != nil { return source }
        let cacheKey = (language + ":" + source) as NSString
        if let value = cache.object(forKey: cacheKey) { return value as String }
        let range = NSRange(source.startIndex..., in: source)
        for pattern in english ? patterns : reversePatterns {
            guard let match = pattern.expression.firstMatch(in: source, range: range) else { continue }
            var arguments: [String: String] = [:]
            for (index, slot) in pattern.slots.enumerated() {
                if let part = Range(match.range(at: index + 1), in: source) { arguments[slot] = String(source[part]) }
            }
            let value = substitute(pattern.target, arguments)
            cache.countLimit = 2048; cache.setObject(value as NSString, forKey: cacheKey)
            return value
        }
        if source.contains("\n") { return source.components(separatedBy: "\n").map(message).joined(separator: "\n") }
        return source
    }
    static func substitute(_ template: String, _ arguments: [String: String]) -> String {
        var output = template
        for match in parameter.matches(in: template, range: NSRange(template.startIndex..., in: template)).reversed() {
            guard let part = Range(match.range, in: output), let slot = Range(match.range(at: 1), in: template), let value = arguments[String(template[slot])] else { continue }
            output.replaceSubrange(part, with: value)
        }
        return output
    }
}

func L(_ source: String, _ arguments: Any...) -> String {
    let values = Dictionary(uniqueKeysWithValues: arguments.enumerated().map { (String($0.offset), String(describing: $0.element)) })
    if AieyesLocale.language == "en", !arguments.isEmpty, LocalizationCatalog.english[source] != nil {
        let rendered = AieyesLocale.substitute(source, values)
        if let exact = LocalizationCatalog.english[rendered] { return exact }
    }
    let template = AieyesLocale.message(source)
    return AieyesLocale.substitute(template, values)
}
