import Foundation

@main struct VerifyProxy {
    static func main() {
        for (url, scheme, host, port) in [
            ("http://127.0.0.1:7890", "http", "127.0.0.1", "7890"),
            ("https://proxy.example", "https", "proxy.example", "443"),
            ("socks5://localhost", "socks5", "localhost", "1080"),
            ("socks5h://[::1]:1080", "socks5h", "::1", "1080")
        ] {
            let draft = ProxyAddressDraft(url: url)
            precondition(draft.scheme == scheme && draft.host == host && draft.port == port)
            let roundTrip = ProxyAddressDraft(url: draft.url)
            precondition(roundTrip.host == host && roundTrip.port == port && roundTrip.scheme == scheme)
        }
        for url in ["http://proxy.example:7890/path?mode=test", "invalid address"] {
            let draft = ProxyAddressDraft(url: url)
            precondition(draft.scheme == "url" && draft.url == url)
        }
        var draft = ProxyAddressDraft(url: "")
        precondition(draft.url == "http://127.0.0.1:7890")
        draft.port = "0"; precondition(draft.url == "invalid-proxy-address")
        draft.port = "65536"; precondition(draft.url == "invalid-proxy-address")
        draft.port = "7890"; draft.host = "bad/host"; precondition(draft.url == "invalid-proxy-address")
        print("Proxy protocol, IPv6, defaults and custom URL checks passed")
    }
}
