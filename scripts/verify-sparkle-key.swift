import Foundation
import CryptoKit
// Read keys from stdin; never print them or modify the login Keychain.
guard let seedLine = readLine(), let publicLine = readLine(),
      let seed = Data(base64Encoded: seedLine), let expected = Data(base64Encoded: publicLine),
      let key = try? Curve25519.Signing.PrivateKey(rawRepresentation: seed),
      key.publicKey.rawRepresentation == expected else { exit(1) }
