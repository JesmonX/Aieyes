import Foundation
import Combine

@MainActor final class PriceReadGate {
    var calls = 0
    var continuation: CheckedContinuation<[ModelPrice], Error>?
    func read() async throws -> [ModelPrice] {
        calls += 1
        return try await withCheckedThrowingContinuation { continuation = $0 }
    }
    func wait(_ count: Int) async {
        for _ in 0..<10000 { if calls >= count { return }; await Task.yield() }
        preconditionFailure("Price read did not start")
    }
    func finish(_ result: Result<[ModelPrice], Error>) { let reply = continuation; continuation = nil; reply?.resume(with: result) }
}
@MainActor final class ReadStarted {
    var started = false
    var continuation: CheckedContinuation<Void, Never>?
    func signal() { started = true; continuation?.resume(); continuation = nil }
    func wait() async { if !started { await withCheckedContinuation { continuation = $0 } } }
}
@main struct VerifyPrices {
    @MainActor static func main() async {
        let gate = PriceReadGate()
        let catalog = PriceCatalog(fetch: { try await gate.read() })
        let old = ModelPrice(id: "vendor/old", name: "First model", input: 1e-6)
        let new = ModelPrice(id: "vendor/new", name: "Second model", input: 2e-6)
        let first = Task { await catalog.load() }
        await gate.wait(1)
        let forced = Task { await catalog.load(force: true) }
        await Task.yield()
        gate.finish(.success([old]))
        await gate.wait(2)
        gate.finish(.success([new]))
        await first.value; await forced.value
        precondition(catalog.prices == [new] && catalog.loaded && !catalog.loading)
        await catalog.load(); precondition(gate.calls == 2, "Cache must survive re-entry")
        catalog.query = " SECOND \n"; precondition(catalog.filtered == [new])
        catalog.query = "missing"; precondition(catalog.filtered.isEmpty)
        catalog.query = "VENDOR/NEW"; precondition(catalog.filtered == [new])
        let failure = Task { await catalog.load(force: true) }
        await gate.wait(3); gate.finish(.failure(ClientError.message("Read failed"))); await failure.value
        precondition(catalog.prices == [new] && catalog.error == "Read failed" && !catalog.loaded)
        let retry = Task { await catalog.load() }
        await gate.wait(4); gate.finish(.success([old])); await retry.value
        precondition(catalog.error == nil && catalog.loaded && catalog.prices == [old])
        var publications = 0
        let subscription = catalog.$prices.dropFirst().sink { _ in publications += 1 }
        catalog.replace([old]); precondition(publications == 0, "Unchanged catalogue should not publish")
        withExtendedLifetime(subscription) {}
        let model = AppModel(autostart: false)
        var visibilityChanges = 0
        let visibility = model.$visibleWindows.dropFirst().sink { _ in visibilityChanges += 1 }
        model.setWindowVisible(true, window: "panel"); model.setWindowVisible(true, window: "panel")
        model.setWindowVisible(false, window: "panel"); model.setWindowVisible(false, window: "panel")
        precondition(visibilityChanges == 2 && !model.isWindowVisible("panel"))
        withExtendedLifetime(visibility) {}
        if ProcessInfo.processInfo.environment["AIEYES_PRICING_ISOLATION"] == "1" {
            let engine = EngineClient(), started = ReadStarted()
            let blocking = Task { try await engine.call("test.block", as: Acknowledgement.self, onProgress: { _ in Task { @MainActor in started.signal() } }) }
            await started.wait()
            let independent = PriceCatalog()
            let before = Date(); await independent.load()
            precondition(independent.prices.first?.id == "local/fixture" && Date().timeIntervalSince(before) < 0.3, "Price browsing queued behind an unrelated blocking request")
            _ = try! await blocking.value
            print("Independent price read completed under 300 ms while the main IPC stream was blocked for 1.5 s")
        }
        print("Price cache, in-flight invalidation, filtering, retained data on failure, retry and visibility deduplication passed")
    }
}
